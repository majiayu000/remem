use std::ffi::OsStr;
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use super::condition::apply_condition;
use super::fixture::{load_fixture, selected_conditions, selected_tasks, validate_relative_path};
use super::isolation::{prepare_codex_isolation, runner_isolation_violation};
use super::run_plan::randomized_run_plan;
use super::score::{
    parse_changed_paths, parse_codex_jsonl_usage, summarize_runs, unauthorized_paths,
};
use super::types::{
    BenchCondition, BenchTokenUsage, CodingBenchFixture, CodingBenchOptions, CodingBenchReport,
    CodingBenchTask, CommandReport, ConditionReport, RunArtifacts, RunReport, RunnerReport,
};

pub fn dry_run_plan(options: &CodingBenchOptions) -> Result<String> {
    let fixture = load_fixture(&options.fixture_path)?;
    let conditions = selected_conditions(options)?;
    let tasks = selected_tasks(&fixture, options)?;
    let total = conditions.len() * tasks.len() * options.runs_per_condition;
    let mut output = String::new();
    output.push_str("coding benchmark dry run\n");
    output.push_str(&format!("fixture: {}\n", options.fixture_path));
    output.push_str(&format!(
        "runs_per_condition: {}\n",
        options.runs_per_condition
    ));
    output.push_str(&format!(
        "runner: {} model: {}\n",
        options.runner, options.model
    ));
    output.push_str(&format!("planned_runs: {total}\n"));
    for condition in &conditions {
        for task in &tasks {
            output.push_str(&format!(
                "- {} {} x{}\n",
                condition.as_str(),
                task.id,
                options.runs_per_condition
            ));
        }
    }
    Ok(output)
}

pub fn run_coding_bench(options: &CodingBenchOptions) -> Result<CodingBenchReport> {
    if options.runs_per_condition == 0 {
        bail!("--runs-per-condition must be greater than zero");
    }
    let fixture = load_fixture(&options.fixture_path)?;
    let conditions = selected_conditions(options)?;
    let tasks = selected_tasks(&fixture, options)?;
    let run_plan = randomized_run_plan(&conditions, tasks.len(), options.runs_per_condition)?;
    let fixture_sha256 = file_sha256(&options.fixture_path)?;
    let generated_at_epoch = current_epoch();
    let artifact_root = report_artifact_root(&options.json_out, generated_at_epoch)?;
    let runner_version = runner_version(options);
    let mut grouped_runs = conditions
        .iter()
        .map(|condition| (*condition, Vec::new()))
        .collect::<Vec<_>>();

    for entry in run_plan {
        let task = tasks
            .get(entry.task_index)
            .context("coding benchmark run plan referenced missing task")?;
        let run = run_one(
            options,
            &fixture,
            entry.condition,
            task,
            entry.run_index,
            &artifact_root,
        )?;
        eprintln!(
            "[coding-bench] {} {} run {}: resolved={} tokens={}",
            entry.condition.as_str(),
            task.id,
            entry.run_index,
            run.resolved,
            run.usage.total_tokens
        );
        if let Some((_, runs)) = grouped_runs
            .iter_mut()
            .find(|(condition, _)| *condition == entry.condition)
        {
            runs.push(run);
        } else {
            bail!(
                "coding benchmark run plan referenced unselected condition {}",
                entry.condition.as_str()
            );
        }
    }

    let mut condition_reports = Vec::new();
    for (condition, runs) in grouped_runs {
        let summary = summarize_runs(&runs);
        condition_reports.push(ConditionReport {
            name: condition,
            summary,
            runs,
        });
    }

    Ok(CodingBenchReport {
        schema_version: 1,
        generated_at_epoch,
        fixture_path: options.fixture_path.clone(),
        fixture_sha256,
        remem_rev: current_git_rev(Path::new(".")).unwrap_or_else(|| "unknown".to_string()),
        source_dirty: current_git_dirty(Path::new(".")),
        command: report_command(options),
        artifact_policy: "raw_artifacts_local_ignored".to_string(),
        runner: RunnerReport {
            provider: options
                .provider
                .clone()
                .unwrap_or_else(|| options.runner.clone()),
            model: options.model.clone(),
            runner: options.runner.clone(),
            version: runner_version,
        },
        runs_per_condition: options.runs_per_condition,
        ignore_budget: options.ignore_budget,
        conditions: condition_reports,
    })
}

fn run_one(
    options: &CodingBenchOptions,
    fixture: &CodingBenchFixture,
    condition: BenchCondition,
    task: &CodingBenchTask,
    run_index: usize,
    artifact_root: &Path,
) -> Result<RunReport> {
    let start = Instant::now();
    let run_root = unique_temp_dir(condition, &task.id, run_index);
    let repo_dir = run_root.join("repo");
    let data_dir = run_root.join("remem-data");
    let artifact_dir =
        artifact_root.join(format!("{}-{}-{}", condition.as_str(), task.id, run_index));
    fs::create_dir_all(&artifact_dir)
        .with_context(|| format!("create artifact dir {}", artifact_dir.display()))?;
    prepare_repo(fixture, &repo_dir)?;
    let setup = apply_condition(condition, fixture, task, &repo_dir, &data_dir)?;
    commit_condition_inputs(&repo_dir)?;
    let prompt = build_prompt(task, setup.prompt_note.as_deref());

    let runner_outcome = invoke_agent(
        options,
        &repo_dir,
        &run_root,
        &setup.env,
        &prompt,
        task.timeout_ms,
    )
    .context("invoke coding-agent runner")?;
    let runner_stdout = write_artifact(&artifact_dir, "runner.stdout", &runner_outcome.stdout)?;
    let runner_stderr = write_artifact(&artifact_dir, "runner.stderr", &runner_outcome.stderr)?;
    let (usage, turns) = if options.runner == "codex" {
        parse_codex_jsonl_usage(&runner_outcome.stdout)
    } else {
        (BenchTokenUsage::default(), None)
    };

    let status = command_output("git", ["status", "--porcelain"], &repo_dir, &[], 30_000)?;
    let changed_paths = parse_changed_paths(&status.stdout);
    let unauthorized = unauthorized_paths(&changed_paths, &task.allowed_paths);
    let diff = command_output("git", ["diff", "--binary"], &repo_dir, &[], 30_000)?;
    let final_diff = write_artifact(&artifact_dir, "final.diff", &diff.stdout)?;
    write_hidden_files(task, &repo_dir)?;

    let mut score_commands = Vec::new();
    let mut score_failed = false;
    for (index, command) in task.score.commands.iter().enumerate() {
        let (program, args) = command
            .split_first()
            .context("score command unexpectedly empty after validation")?;
        let outcome = command_output(
            program,
            args.iter().map(String::as_str),
            &repo_dir,
            &[],
            120_000,
        )
        .with_context(|| format!("run score command {:?}", command))?;
        if outcome.exit_code != Some(0) || outcome.timed_out {
            score_failed = true;
        }
        let stdout_artifact = write_artifact(
            &artifact_dir,
            &format!("score-{index}.stdout"),
            &outcome.stdout,
        )?;
        let stderr_artifact = write_artifact(
            &artifact_dir,
            &format!("score-{index}.stderr"),
            &outcome.stderr,
        )?;
        score_commands.push(CommandReport {
            command: command.clone(),
            exit_code: outcome.exit_code,
            timed_out: outcome.timed_out,
            stdout_artifact,
            stderr_artifact,
        });
    }

    let final_head_sha = current_git_rev(&repo_dir);
    let failure_reason = failure_reason(
        score_failed,
        &unauthorized,
        runner_isolation_violation(&runner_outcome.stdout, &runner_outcome.stderr).as_deref(),
        runner_outcome.timed_out,
        runner_outcome.exit_code,
    );
    let resolved = failure_reason.is_none();
    if !options.keep_workdirs {
        let _ = fs::remove_dir_all(&run_root);
    }

    Ok(RunReport {
        condition,
        task_id: task.id.clone(),
        run_index,
        resolved,
        failure_reason,
        usage,
        turns,
        wall_time_ms: start.elapsed().as_millis(),
        final_head_sha,
        changed_paths,
        unauthorized_path_changes: unauthorized,
        runner_exit_code: runner_outcome.exit_code,
        runner_timed_out: runner_outcome.timed_out,
        score_commands,
        artifacts: RunArtifacts {
            runner_stdout,
            runner_stderr,
            final_diff,
        },
        workdir: options
            .keep_workdirs
            .then(|| run_root.to_string_lossy().to_string()),
    })
}

fn prepare_repo(fixture: &CodingBenchFixture, repo_dir: &Path) -> Result<()> {
    fs::create_dir_all(repo_dir)
        .with_context(|| format!("create repo dir {}", repo_dir.display()))?;
    for (path, content) in &fixture.repo.files {
        write_relative_file(repo_dir, path, content)?;
    }
    let init = command_output("git", ["init", "-b", "main"], repo_dir, &[], 30_000)?;
    if init.exit_code != Some(0) {
        let fallback = command_output("git", ["init"], repo_dir, &[], 30_000)?;
        ensure_success("git init", &fallback)?;
        ensure_success(
            "git checkout -b main",
            &command_output("git", ["checkout", "-b", "main"], repo_dir, &[], 30_000)?,
        )?;
    }
    ensure_success(
        "git config user.email",
        &command_output(
            "git",
            ["config", "user.email", "coding-bench@example.invalid"],
            repo_dir,
            &[],
            30_000,
        )?,
    )?;
    ensure_success(
        "git config user.name",
        &command_output(
            "git",
            ["config", "user.name", "remem coding bench"],
            repo_dir,
            &[],
            30_000,
        )?,
    )?;
    ensure_success(
        "git add",
        &command_output("git", ["add", "."], repo_dir, &[], 30_000)?,
    )?;
    ensure_success(
        "git commit",
        &command_output(
            "git",
            ["commit", "-m", "initial fixture"],
            repo_dir,
            &[],
            30_000,
        )?,
    )?;
    Ok(())
}

fn commit_condition_inputs(repo_dir: &Path) -> Result<()> {
    let status = command_output("git", ["status", "--porcelain"], repo_dir, &[], 30_000)?;
    if status.stdout.trim().is_empty() {
        return Ok(());
    }
    ensure_success(
        "git add condition inputs",
        &command_output("git", ["add", "."], repo_dir, &[], 30_000)?,
    )?;
    ensure_success(
        "git commit condition inputs",
        &command_output(
            "git",
            ["commit", "-m", "condition inputs"],
            repo_dir,
            &[],
            30_000,
        )?,
    )?;
    Ok(())
}

fn invoke_agent(
    options: &CodingBenchOptions,
    repo_dir: &Path,
    run_root: &Path,
    env: &[(String, String)],
    prompt: &str,
    timeout_ms: u64,
) -> Result<CommandOutcome> {
    match options.runner.as_str() {
        "codex" => {
            let isolation = prepare_codex_isolation(run_root, &options.codex_bin)?;
            let mut runner_env = env.to_vec();
            runner_env.extend(isolation.env.clone());
            let args = build_codex_exec_args(options, repo_dir, prompt);
            let mut wrapped_args = isolation.args_prefix.clone();
            wrapped_args.extend(args);
            let outcome = command_output(
                &isolation.program,
                wrapped_args.iter().map(String::as_str),
                repo_dir,
                &runner_env,
                timeout_ms,
            );
            isolation.cleanup();
            outcome
        }
        "noop" => Ok(CommandOutcome {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
            timed_out: false,
        }),
        other => bail!("unsupported coding benchmark runner: {other}"),
    }
}

fn build_codex_exec_args(
    options: &CodingBenchOptions,
    repo_dir: &Path,
    prompt: &str,
) -> Vec<String> {
    let mut args = vec![
        "exec".to_string(),
        "--json".to_string(),
        "--color".to_string(),
        "never".to_string(),
        "--ignore-user-config".to_string(),
        "--ignore-rules".to_string(),
        "--ephemeral".to_string(),
        "--disable".to_string(),
        "hooks".to_string(),
        "--cd".to_string(),
        repo_dir.to_string_lossy().to_string(),
        "--model".to_string(),
        options.model.clone(),
        "--sandbox".to_string(),
        "danger-full-access".to_string(),
    ];
    if !options.reasoning_effort.trim().is_empty() {
        args.push("-c".to_string());
        args.push(format!(
            "model_reasoning_effort=\"{}\"",
            toml_escape(&options.reasoning_effort)
        ));
    }
    if let Some(provider) = options.provider.as_deref() {
        args.push("-c".to_string());
        args.push(format!("model_provider=\"{}\"", toml_escape(provider)));
    }
    args.push(prompt.to_string());
    args
}

fn build_prompt(task: &CodingBenchTask, condition_note: Option<&str>) -> String {
    let mut prompt = String::new();
    prompt.push_str("You are running an isolated coding benchmark task.\n");
    if let Some(note) = condition_note {
        prompt.push_str(note);
        prompt.push('\n');
    }
    prompt.push_str("Only inspect files under the repository root and context files explicitly named above. Do not inspect environment variables, parent directories, CODEX_HOME, HOME, tool caches, benchmark harness artifacts, or hidden tests.\n");
    prompt.push_str("Modify the repository to satisfy the task. Do not inspect or depend on hidden tests. Keep edits scoped to the task.\n\n");
    prompt.push_str("Task:\n");
    prompt.push_str(&task.prompt);
    prompt.push('\n');
    prompt
}

fn write_hidden_files(task: &CodingBenchTask, repo_dir: &Path) -> Result<()> {
    for (path, content) in &task.score.hidden_files {
        write_relative_file(repo_dir, path, content)?;
    }
    Ok(())
}

fn write_relative_file(root: &Path, relative: &str, content: &str) -> Result<()> {
    validate_relative_path(relative)?;
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create parent directory {}", parent.display()))?;
    }
    fs::write(&path, content).with_context(|| format!("write {}", path.display()))
}

fn command_output<I, S>(
    program: &str,
    args: I,
    cwd: &Path,
    env: &[(String, String)],
    timeout_ms: u64,
) -> Result<CommandOutcome>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("REMEM_DATA_DIR")
        .env_remove("REMEM_CIPHER_KEY")
        .env_remove("REMEM_DISABLE_HOOKS")
        .env_remove("REMEM_ALLOW_PLAINTEXT_DB")
        .env_remove("CODEX_HOME")
        .env_remove("CODEX_THREAD_ID")
        .env_remove("VIRTUAL_ENV")
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .env_remove("CONDA_PREFIX")
        .env_remove("CONDA_DEFAULT_ENV")
        .env_remove("PIPENV_ACTIVE")
        .env_remove("POETRY_ACTIVE")
        .env_remove("PYENV_VERSION");
    for (key, value) in env {
        command.env(key, value);
    }
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("spawn command {program}"))?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut timed_out = false;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            terminate_command(&mut child);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("wait for command {program}"))?;
    Ok(CommandOutcome {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code(),
        timed_out,
    })
}

fn terminate_command(child: &mut Child) {
    #[cfg(unix)]
    {
        let process_group = -(child.id() as libc::pid_t);
        terminate_unix_process_group("TERM", libc::SIGTERM, process_group);
        std::thread::sleep(Duration::from_millis(200));
        match child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => terminate_unix_process_group("KILL", libc::SIGKILL, process_group),
            Err(err) => eprintln!(
                "[coding-bench] failed to poll timed-out runner process {}: {err}",
                child.id()
            ),
        }
    }
    #[cfg(not(unix))]
    {
        if let Err(err) = child.kill() {
            eprintln!(
                "[coding-bench] failed to kill timed-out runner process {}: {err}",
                child.id()
            );
        }
    }
}

#[cfg(unix)]
fn terminate_unix_process_group(
    signal_name: &str,
    signal: libc::c_int,
    process_group: libc::pid_t,
) {
    if unsafe { libc::kill(process_group, signal) } != 0 {
        let err = std::io::Error::last_os_error();
        eprintln!(
            "[coding-bench] failed to send {signal_name} to process group {process_group}: {err}"
        );
    }
}

fn ensure_success(label: &str, outcome: &CommandOutcome) -> Result<()> {
    if outcome.exit_code == Some(0) && !outcome.timed_out {
        return Ok(());
    }
    bail!(
        "{label} failed with exit={:?} timed_out={} stderr={}",
        outcome.exit_code,
        outcome.timed_out,
        outcome.stderr
    )
}

fn write_artifact(dir: &Path, name: &str, content: &str) -> Result<String> {
    let path = dir.join(name);
    let mut file =
        fs::File::create(&path).with_context(|| format!("create artifact {}", path.display()))?;
    file.write_all(content.as_bytes())
        .with_context(|| format!("write artifact {}", path.display()))?;
    Ok(path.to_string_lossy().to_string())
}

fn report_artifact_root(json_out: &str, epoch: i64) -> Result<PathBuf> {
    let parent = Path::new(json_out)
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let path = parent.join("artifacts").join(epoch.to_string());
    fs::create_dir_all(&path)
        .with_context(|| format!("create artifact root {}", path.display()))?;
    Ok(path)
}

fn unique_temp_dir(condition: BenchCondition, task_id: &str, run_index: usize) -> PathBuf {
    std::env::temp_dir().join(format!(
        "remem-coding-bench-{}-{}-{}-{}-{}",
        condition.as_str(),
        task_id,
        run_index,
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ))
}

fn current_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn current_git_rev(cwd: &Path) -> Option<String> {
    let outcome = command_output("git", ["rev-parse", "HEAD"], cwd, &[], 30_000).ok()?;
    (outcome.exit_code == Some(0)).then(|| outcome.stdout.trim().to_string())
}

fn runner_version(options: &CodingBenchOptions) -> Option<String> {
    if options.runner != "codex" {
        return None;
    }
    let outcome = command_output(
        &options.codex_bin,
        ["--version"],
        Path::new("."),
        &[],
        30_000,
    )
    .ok()?;
    (outcome.exit_code == Some(0)).then(|| outcome.stdout.trim().to_string())
}

fn file_sha256(path: &str) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("read fixture for sha256 {path}"))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn current_git_dirty(cwd: &Path) -> Option<bool> {
    let outcome = command_output("git", ["status", "--porcelain"], cwd, &[], 30_000).ok()?;
    (outcome.exit_code == Some(0)).then(|| !outcome.stdout.trim().is_empty())
}

fn report_command(options: &CodingBenchOptions) -> Vec<String> {
    let mut command = vec![
        "remem".to_string(),
        "eval-coding-bench".to_string(),
        "--fixture".to_string(),
        options.fixture_path.clone(),
        "--runs-per-condition".to_string(),
        options.runs_per_condition.to_string(),
        "--runner".to_string(),
        options.runner.clone(),
        "--model".to_string(),
        options.model.clone(),
        "--reasoning-effort".to_string(),
        options.reasoning_effort.clone(),
        "--json-out".to_string(),
        options.json_out.clone(),
    ];
    if options.codex_bin != "codex" {
        command.push("--codex-bin".to_string());
        command.push(options.codex_bin.clone());
    }
    if let Some(provider) = &options.provider {
        command.push("--provider".to_string());
        command.push(provider.clone());
    }
    if let Some(condition) = &options.condition {
        command.push("--condition".to_string());
        command.push(condition.clone());
    }
    if let Some(task) = &options.task {
        command.push("--task".to_string());
        command.push(task.clone());
    }
    if options.ignore_budget {
        command.push("--ignore-budget".to_string());
    }
    if options.keep_workdirs {
        command.push("--keep-workdirs".to_string());
    }
    command
}

fn failure_reason(
    score_failed: bool,
    unauthorized: &[String],
    runner_isolation_violation: Option<&str>,
    runner_timed_out: bool,
    runner_exit_code: Option<i32>,
) -> Option<String> {
    if let Some(reason) = runner_isolation_violation {
        return Some(reason.to_string());
    }
    if runner_timed_out {
        return Some("runner timed out".to_string());
    }
    if !unauthorized.is_empty() {
        return Some(format!(
            "unauthorized path changes: {}",
            unauthorized.join(", ")
        ));
    }
    if score_failed {
        return Some("score command failed".to_string());
    }
    if runner_exit_code.is_none() {
        return Some("runner terminated by signal".to_string());
    }
    None
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[derive(Debug)]
struct CommandOutcome {
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    timed_out: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_expanded_default_matrix() -> Result<()> {
        let fixture = load_fixture("eval/coding-bench/fixtures/tasks.json")?;
        let options = CodingBenchOptions {
            fixture_path: "eval/coding-bench/fixtures/tasks.json".to_string(),
            runs_per_condition: 3,
            json_out: "/tmp/remem-coding-bench.json".to_string(),
            condition: None,
            task: None,
            keep_workdirs: false,
            dry_run: true,
            runner: "noop".to_string(),
            codex_bin: "codex".to_string(),
            model: "gpt-5.5".to_string(),
            provider: None,
            reasoning_effort: "medium".to_string(),
            ignore_budget: false,
        };
        let conditions = selected_conditions(&options)?;
        let tasks = selected_tasks(&fixture, &options)?;
        assert_eq!(conditions.len(), 3);
        assert!(tasks.len() >= 5);
        assert_eq!(
            conditions.len() * tasks.len() * options.runs_per_condition,
            45
        );
        Ok(())
    }

    #[test]
    fn codex_runner_ignores_host_config_rules_hooks_and_session_files() {
        let options = CodingBenchOptions {
            fixture_path: "eval/coding-bench/fixtures/tasks.json".to_string(),
            runs_per_condition: 1,
            json_out: "/tmp/remem-coding-bench.json".to_string(),
            condition: None,
            task: None,
            keep_workdirs: false,
            dry_run: false,
            runner: "codex".to_string(),
            codex_bin: "codex".to_string(),
            model: "gpt-5.5".to_string(),
            provider: Some("codexapi".to_string()),
            reasoning_effort: "medium".to_string(),
            ignore_budget: true,
        };
        let args = build_codex_exec_args(&options, Path::new("/tmp/remem-bench-repo"), "prompt");

        assert!(args.contains(&"--ignore-user-config".to_string()));
        assert!(args.contains(&"--ignore-rules".to_string()));
        assert!(args.contains(&"--ephemeral".to_string()));
        assert!(args
            .windows(2)
            .any(|window| window == ["--disable", "hooks"]));
        assert!(args
            .windows(2)
            .any(|window| window == ["--sandbox", "danger-full-access"]));
        assert!(!args.contains(&"--dangerously-bypass-approvals-and-sandbox".to_string()));
        assert!(!args.contains(&"--dangerously-bypass-hook-trust".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn command_output_timeout_terminates_process_group_children() -> Result<()> {
        let start = Instant::now();
        let outcome = command_output("sh", ["-c", "sleep 10 & wait"], Path::new("."), &[], 100)?;

        assert!(outcome.timed_out);
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "timeout should not wait for a grandchild sleep to exit"
        );
        Ok(())
    }
}
