use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{ensure, Context, Result};
use remem::rules::{
    artifact_path_for_project, classify_preference_predicate, evaluate_pre_tool_use,
    write_artifact_atomic, CompiledRule, CompiledRulesArtifact, PreferencePredicate, RuleAction,
    RuleOverrideState, RulePredicate,
};
use serde::Deserialize;
use serde_json::json;

const FIXTURES: &str = include_str!("fixtures/rule-enforcement-repeated-corrections.json");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureSuite {
    schema_version: u32,
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    id: String,
    correction: String,
    source_memory_id: i64,
    reinforcement_count: i64,
    violating_command: String,
    compliant_command: String,
}

#[test]
fn repeated_correction_fixtures_warn_only_with_compiled_rules() -> Result<()> {
    let suite = load_fixtures()?;
    ensure!(suite.schema_version == 1, "unsupported fixture schema");
    ensure!(suite.scenarios.len() == 3, "expected three fixture classes");

    let root = test_dir("rule-enforcement-fixtures");
    for scenario in suite.scenarios {
        ensure!(!scenario.correction.trim().is_empty(), "blank correction");
        ensure!(
            scenario.reinforcement_count >= 3,
            "{} is not repeatedly reinforced",
            scenario.id
        );
        let project_dir = root.join(&scenario.id);
        std::fs::create_dir_all(&project_dir)?;
        let project = remem::db::project_from_cwd(&project_dir.to_string_lossy());
        let data_dir = project_dir.join("data");
        let artifact_path = artifact_path_for_project(&data_dir, &project);

        write_artifact_atomic(&artifact_path, &CompiledRulesArtifact::new(1, Vec::new()))?;
        let without_rule = evaluate_pre_tool_use(
            &hook_input(&project_dir, &scenario.id, &scenario.violating_command),
            Some("claude-code"),
            &data_dir,
            true,
        )?;
        ensure!(
            without_rule.output.is_none() && without_rule.diagnostics.is_empty(),
            "{} warned without a compiled rule",
            scenario.id
        );

        let rule = compiled_rule(&scenario)?;
        write_artifact_atomic(&artifact_path, &CompiledRulesArtifact::new(2, vec![rule]))?;
        let violation = evaluate_pre_tool_use(
            &hook_input(&project_dir, &scenario.id, &scenario.violating_command),
            Some("claude-code"),
            &data_dir,
            true,
        )?;
        let output = violation
            .output
            .with_context(|| format!("{} did not warn on violation", scenario.id))?;
        ensure!(
            output["systemMessage"]
                .as_str()
                .is_some_and(|message| message.contains("warning")),
            "{} did not emit a visible warning",
            scenario.id
        );
        ensure!(
            output["hookSpecificOutput"]
                .get("permissionDecision")
                .is_none(),
            "{} defaulted to block instead of warn",
            scenario.id
        );

        let compliant = evaluate_pre_tool_use(
            &hook_input(&project_dir, &scenario.id, &scenario.compliant_command),
            Some("claude-code"),
            &data_dir,
            true,
        )?;
        ensure!(
            compliant.output.is_none() && compliant.diagnostics.is_empty(),
            "{} warned on its compliant command",
            scenario.id
        );
    }
    Ok(())
}

#[test]
fn structural_rules_follow_shell_boundaries_and_force_refspecs() -> Result<()> {
    let suite = load_fixtures()?;
    let force_scenario = suite
        .scenarios
        .iter()
        .find(|scenario| scenario.id == "forbidden-command")
        .context("fixture suite is missing the forbidden-command scenario")?;
    let trailer_scenario = suite
        .scenarios
        .iter()
        .find(|scenario| scenario.id == "forbidden-commit-trailer")
        .context("fixture suite is missing the forbidden-commit-trailer scenario")?;
    let artifact = CompiledRulesArtifact::new(
        2,
        vec![
            compiled_rule(force_scenario)?,
            compiled_rule(trailer_scenario)?,
        ],
    );

    for command in [
        "echo safe\ngit push --force",
        "(git push --force)",
        "{ git push origin main -f; }",
        "! { git push --force; }",
        "{ { git push --force; }; }",
        "(git commit --trailer AI-generated-by=bot)",
        "{\ngit commit --trailer AI-generated-by=bot\n}",
        "git push origin +main:main",
        "git push origin +refs/heads/main:refs/heads/main --",
        "git push --repo origin +HEAD:main",
        "git push origin -- +HEAD:main",
        "git push \\\n--force",
        "git push \"--force\"",
        "git push --mirror origin",
        "env GIT_SSH_COMMAND=ssh command git push --force",
        "$'git' push $'--force'",
        "echo \"$(git push --force)\"",
        "echo $(( $(git push --force) + 1 ))",
        "{ echo $(( $(git push --force) + 1 )); }",
        "(( $(git push --force) + 1 ))",
        "for (( i = $(git push --force); i < 1; i++ )); do echo ok; done",
        "FOO=$(git push --force) true",
        "cat <<EOF\n$(git push --force)\nEOF",
        "bash -c 'git push --force'",
        "/bin/sh -ec 'git push --mirror origin'",
        "env FLAG=1 command bash -lc 'git push --force'",
        "git push --{force,force}",
        "printf '%s\\n' {1..257}; git push --force",
        "cat <<EOF\ngit push --force\nEOF\ngit push --force",
        "echo safe # <<EOF\ngit push --force",
        "cat <<< 'git push --force'\ngit push --force",
        "cat <<A <<'B'\nfirst\nA\ngit push --force\nB\ngit push --force",
    ] {
        let outcome = remem::rules::evaluate_artifact(
            &artifact,
            &remem::rules::EvaluationInput {
                command: command.to_string(),
            },
        );
        ensure!(
            outcome.matches.len() == 1 && outcome.diagnostics.is_empty(),
            "expected one structural match for {command:?}, got {outcome:?}"
        );
    }

    for command in [
        "echo git push --force",
        "echo '(git push --force)'",
        "echo \"{ git push --force; }\"",
        "echo 'safe\ngit push --force'",
        "git push +server main",
        "git push origin +",
        "git push origin +:main",
        "git push origin :main",
        "git push -o +main:main origin main",
        "git push --push-option +main:main origin main",
        "echo {git push --force}",
        "echo }",
        "echo $((1 << 2))",
        "f() { git push --force; }",
        "command -v git push --force",
        "env NOTE=example echo git push --force",
        "cat <<EOF\ngit push --force\nEOF",
        "cat <<'EOF'\ngit push --force\nEOF",
        "cat <<'EOF'\n$(git push --force)\nEOF",
        "cat <<-EOF\n\tgit push --force\n\tEOF",
        "cat <<< 'git push --force'",
        "bash -c \"$PAYLOAD\"",
        "git push '--{force,force}'",
    ] {
        let outcome = remem::rules::evaluate_artifact(
            &artifact,
            &remem::rules::EvaluationInput {
                command: command.to_string(),
            },
        );
        ensure!(
            outcome.matches.is_empty() && outcome.diagnostics.is_empty(),
            "expected no structural match for {command:?}, got {outcome:?}"
        );
    }

    let invalid = remem::rules::evaluate_artifact(
        &artifact,
        &remem::rules::EvaluationInput {
            command: "echo { git push --force; }".to_string(),
        },
    );
    ensure!(
        invalid.matches.is_empty()
            && invalid.diagnostics.len() == 2
            && invalid
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.message.contains("parse error")),
        "invalid Bash must fail open with rule diagnostics, got {invalid:?}"
    );

    Ok(())
}

#[test]
#[ignore = "manual p95 harness; run under --release with --ignored --nocapture"]
fn rule_hook_cli_p95_meets_absolute_budgets() -> Result<()> {
    const MAX_DELTA_MS: f64 = 1.0;
    const MAX_ENABLED_P95_MS: f64 = 15.0;

    let suite = load_fixtures()?;
    let root = test_dir("rule-hook-latency");
    let project_dir = root.join("project");
    std::fs::create_dir_all(&project_dir)?;
    let project = remem::db::project_from_cwd(&project_dir.to_string_lossy());
    let baseline = HookProcess::new(&root.join("baseline"), false, &project_dir)?;
    let enabled_empty = HookProcess::new(&root.join("enabled-empty"), true, &project_dir)?;
    let enabled_non_regex = HookProcess::new(&root.join("enabled-non-regex"), true, &project_dir)?;
    let enabled = HookProcess::new(&root.join("enabled"), true, &project_dir)?;
    let rules = suite
        .scenarios
        .iter()
        .map(compiled_rule)
        .collect::<Result<Vec<_>>>()?;
    let non_regex_rule = rules
        .iter()
        .find(|rule| matches!(rule.predicate, RulePredicate::CommitTrailerForbidden { .. }))
        .cloned()
        .context("fixture suite is missing a non-regex rule")?;
    write_artifact_atomic(
        artifact_path_for_project(&enabled.data_dir, &project),
        &CompiledRulesArtifact::new(2, rules),
    )?;
    write_artifact_atomic(
        artifact_path_for_project(&enabled_empty.data_dir, &project),
        &CompiledRulesArtifact::new(2, Vec::new()),
    )?;
    write_artifact_atomic(
        artifact_path_for_project(&enabled_non_regex.data_dir, &project),
        &CompiledRulesArtifact::new(2, vec![non_regex_rule]),
    )?;

    let probe = enabled.run("npm install left-pad", true)?;
    ensure!(
        String::from_utf8(probe.stdout)?.contains("warning"),
        "enabled benchmark path did not exercise its compiled artifact"
    );
    for _ in 0..10 {
        baseline.run("cargo check", false)?;
        enabled_empty.run("cargo check", false)?;
        enabled_non_regex.run("cargo check", false)?;
        enabled.run("cargo check", false)?;
        enabled.run("cat <<A <<B\none\nA\ntwo\nB\ngit push --force", false)?;
    }

    let mut baseline_a = Vec::with_capacity(60);
    let mut baseline_b = Vec::with_capacity(60);
    let mut enabled_empty_samples = Vec::with_capacity(120);
    let mut enabled_non_regex_samples = Vec::with_capacity(120);
    let mut enabled_a = Vec::with_capacity(60);
    let mut enabled_b = Vec::with_capacity(60);
    let mut complex_ast_samples = Vec::with_capacity(60);
    for _ in 0..60 {
        baseline_a.push(baseline.run("cargo check", false)?.elapsed);
        enabled_empty_samples.push(enabled_empty.run("cargo check", false)?.elapsed);
        enabled_non_regex_samples.push(enabled_non_regex.run("cargo check", false)?.elapsed);
        enabled_a.push(enabled.run("cargo check", false)?.elapsed);
        enabled_b.push(enabled.run("cargo check", false)?.elapsed);
        enabled_non_regex_samples.push(enabled_non_regex.run("cargo check", false)?.elapsed);
        enabled_empty_samples.push(enabled_empty.run("cargo check", false)?.elapsed);
        baseline_b.push(baseline.run("cargo check", false)?.elapsed);
        complex_ast_samples.push(
            enabled
                .run("cat <<A <<B\none\nA\ntwo\nB\ngit push --force", false)?
                .elapsed,
        );
    }

    let baseline_a_p95 = percentile_ms(&baseline_a, 95);
    let baseline_b_p95 = percentile_ms(&baseline_b, 95);
    let enabled_a_p95 = percentile_ms(&enabled_a, 95);
    let enabled_b_p95 = percentile_ms(&enabled_b, 95);
    let enabled_empty_p95 = percentile_ms(&enabled_empty_samples, 95);
    let enabled_non_regex_p95 = percentile_ms(&enabled_non_regex_samples, 95);
    let baseline_p95 = baseline_a_p95.max(baseline_b_p95);
    let enabled_p95 = enabled_a_p95.max(enabled_b_p95);
    let observed_noise_ms = (baseline_a_p95 - baseline_b_p95).abs();
    let mut baseline_samples = baseline_a.clone();
    baseline_samples.extend_from_slice(&baseline_b);
    let mut enabled_samples = enabled_a.clone();
    enabled_samples.extend_from_slice(&enabled_b);
    let measurement_noise_ms = median_absolute_deviation_ms(&baseline_samples)
        .max(median_absolute_deviation_ms(&enabled_samples));
    let delta_ms = enabled_p95 - baseline_p95;
    let complex_ast_p95 = percentile_ms(&complex_ast_samples, 95);

    eprintln!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version": 2,
            "samples_per_primary_cohort": 60,
            "samples_per_diagnostic_cohort": 120,
            "baseline_a_p95_ms": baseline_a_p95,
            "baseline_b_p95_ms": baseline_b_p95,
            "enabled_empty_artifact_p95_ms": enabled_empty_p95,
            "enabled_non_regex_artifact_p95_ms": enabled_non_regex_p95,
            "enabled_a_p95_ms": enabled_a_p95,
            "enabled_b_p95_ms": enabled_b_p95,
            "baseline_p95_ms": baseline_p95,
            "enabled_p95_ms": enabled_p95,
            "delta_ms": delta_ms,
            "max_delta_ms": MAX_DELTA_MS,
            "max_enabled_p95_ms": MAX_ENABLED_P95_MS,
            "delta_within_budget": delta_ms <= MAX_DELTA_MS,
            "enabled_p95_within_budget": enabled_p95 <= MAX_ENABLED_P95_MS,
            "latency_budget_passed": delta_ms <= MAX_DELTA_MS && enabled_p95 <= MAX_ENABLED_P95_MS,
            "observed_baseline_noise_ms": observed_noise_ms,
            "measurement_noise_mad_ms": measurement_noise_ms,
            "delta_within_observed_mad": delta_ms <= measurement_noise_ms,
            "complex_ast_p95_ms": complex_ast_p95,
            "complex_ast_below_ten_percent_of_hook_timeout": complex_ast_p95 < 500.0,
        }))?
    );
    ensure!(
        delta_ms <= MAX_DELTA_MS,
        "enabled p95 delta {delta_ms:.3}ms exceeded the {MAX_DELTA_MS:.3}ms budget"
    );
    ensure!(
        enabled_p95 <= MAX_ENABLED_P95_MS,
        "enabled p95 {enabled_p95:.3}ms exceeded the {MAX_ENABLED_P95_MS:.3}ms hard limit"
    );
    ensure!(
        complex_ast_p95 < 500.0,
        "complex AST p95 {complex_ast_p95:.3}ms exceeded 10% of the 5s hook timeout"
    );
    Ok(())
}

fn load_fixtures() -> Result<FixtureSuite> {
    serde_json::from_str(FIXTURES).context("parse rule enforcement fixtures")
}

fn compiled_rule(scenario: &Scenario) -> Result<CompiledRule> {
    let classification = classify_preference_predicate(&scenario.correction)
        .with_context(|| format!("{} correction did not classify", scenario.id))?;
    let predicate = match classification.predicate {
        PreferencePredicate::CommandRegex { pattern, .. } => RulePredicate::CommandRegex {
            pattern,
            message: "Command violates a compiled preference".to_string(),
        },
        PreferencePredicate::CommitTrailerForbidden { trailer, .. } => {
            RulePredicate::CommitTrailerForbidden {
                trailer,
                message: "Commit message violates a compiled trailer preference".to_string(),
            }
        }
        PreferencePredicate::GitPushForceForbidden { .. } => RulePredicate::GitPushForceForbidden {
            message: "Command violates a compiled forbidden-command preference".to_string(),
        },
    };
    Ok(CompiledRule {
        rule_id: format!("pref-{}-1", scenario.source_memory_id),
        source_memory_id: scenario.source_memory_id,
        reinforcement_count: scenario.reinforcement_count,
        action: RuleAction::Warn,
        override_state: RuleOverrideState {
            disabled: false,
            action_override: None,
        },
        predicate,
    })
}

fn hook_input(project: &Path, session_id: &str, command: &str) -> String {
    json!({
        "session_id": session_id,
        "cwd": project,
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": command}
    })
    .to_string()
}

struct HookProcess {
    data_dir: PathBuf,
    config_path: PathBuf,
    home_dir: PathBuf,
    project_dir: PathBuf,
}

struct HookRun {
    elapsed: Duration,
    stdout: Vec<u8>,
}

impl HookProcess {
    fn new(root: &Path, enabled: bool, project_dir: &Path) -> Result<Self> {
        let data_dir = root.join("data");
        let home_dir = root.join("home");
        std::fs::create_dir_all(&data_dir)?;
        std::fs::create_dir_all(home_dir.join(".config/git"))?;
        let config_path = root.join("config.toml");
        std::fs::write(
            &config_path,
            format!(
                "[rule_compilation]\nenabled = {enabled}\nrule_compile_min_reinforcement = 3\n"
            ),
        )?;
        Ok(Self {
            data_dir,
            config_path,
            home_dir,
            project_dir: project_dir.to_path_buf(),
        })
    }

    fn run(&self, command: &str, capture_stdout: bool) -> Result<HookRun> {
        let started = Instant::now();
        let mut child = Command::new(env!("CARGO_BIN_EXE_remem"))
            .args(["rules", "eval", "--host", "claude-code"])
            .env("REMEM_DATA_DIR", &self.data_dir)
            .env("REMEM_CONFIG", &self.config_path)
            .env("HOME", &self.home_dir)
            .env("XDG_CONFIG_HOME", self.home_dir.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("REMEM_DISABLE_HOOKS")
            .stdin(Stdio::piped())
            .stdout(if capture_stdout {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(Stdio::piped())
            .spawn()
            .context("spawn remem rules eval")?;
        child
            .stdin
            .take()
            .context("rules eval stdin")?
            .write_all(hook_input(&self.project_dir, "latency-session", command).as_bytes())?;
        let output = child
            .wait_with_output()
            .context("wait for remem rules eval")?;
        let elapsed = started.elapsed();
        ensure!(
            output.status.success(),
            "remem rules eval failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(HookRun {
            elapsed,
            stdout: output.stdout,
        })
    }
}

fn percentile_ms(samples: &[Duration], percentile: usize) -> f64 {
    let mut values = samples
        .iter()
        .map(Duration::as_secs_f64)
        .map(|seconds| seconds * 1000.0)
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    let index = (values.len() - 1) * percentile / 100;
    values[index]
}

fn median_absolute_deviation_ms(samples: &[Duration]) -> f64 {
    let mut values = samples
        .iter()
        .map(Duration::as_secs_f64)
        .map(|seconds| seconds * 1000.0)
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    let median = values[values.len() / 2];
    let mut deviations = values
        .into_iter()
        .map(|value| (value - median).abs())
        .collect::<Vec<_>>();
    deviations.sort_by(f64::total_cmp);
    deviations[deviations.len() / 2]
}

fn test_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "remem-{label}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}
