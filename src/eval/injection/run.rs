use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::types::{
    InjectionCaseReport, InjectionEvalMetadata, InjectionEvalOptions, InjectionEvalReport,
    InjectionMetricSummary, InjectionRateMetric, CORPUS_NAME,
};

const PROJECT: &str = "/tmp/remem-injection-eval/repo";
const OTHER_PROJECT: &str = "/tmp/remem-injection-eval/other";
const ABSTENTION_PROJECT: &str = "/tmp/remem-injection-eval/abstain";
const HOST: &str = "codex-cli";
const USER_PROMPT_HOST: &str = "claude-code";
const CURRENT_BRANCH: &str = "main";
const ABSTENTION_FORBIDDEN_TITLE: &str = "Unrelated recent deployment note";
const STALE_ANCHOR_TITLE: &str = "Stale source anchor decision";
const USER_PROMPT_SESSION: &str = "eval-user-prompt-submit";

#[derive(Clone, Copy)]
struct FixtureMemory {
    id: i64,
    project: &'static str,
    topic_key: &'static str,
    title: &'static str,
    content: &'static str,
    memory_type: &'static str,
    branch: Option<&'static str>,
    status: &'static str,
    updated_offset: i64,
    expected: InjectionExpectation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InjectionExpectation {
    Expected,
    Forbidden,
    Filler,
}

const FIXTURE_MEMORIES: &[FixtureMemory] = &[
    FixtureMemory {
        id: 1,
        project: PROJECT,
        topic_key: "inject-migration-locking",
        title: "Migration locking fix",
        content: "Root cause: startup migrations raced. Fix: serialize migration execution and verify with cargo test migrate::tests.",
        memory_type: "bugfix",
        branch: Some(CURRENT_BRANCH),
        status: "active",
        updated_offset: 0,
        expected: InjectionExpectation::Expected,
    },
    FixtureMemory {
        id: 2,
        project: PROJECT,
        topic_key: "inject-api-token-handling",
        title: "API token handling decision",
        content: "Keep API tokens in the temp data directory and avoid reading user-level state during sandboxed evals.",
        memory_type: "decision",
        branch: None,
        status: "active",
        updated_offset: -1,
        expected: InjectionExpectation::Expected,
    },
    FixtureMemory {
        id: 3,
        project: PROJECT,
        topic_key: "inject-branch-mismatch",
        title: "Feature branch wasm snapshot",
        content: "This feature-only memory must not appear when the SessionStart branch is main.",
        memory_type: "decision",
        branch: Some("feature/wasm"),
        status: "active",
        updated_offset: 10,
        expected: InjectionExpectation::Forbidden,
    },
    FixtureMemory {
        id: 4,
        project: OTHER_PROJECT,
        topic_key: "inject-cross-project",
        title: "Other project migration shortcut",
        content: "A different project memory must not leak into this repo's SessionStart context.",
        memory_type: "bugfix",
        branch: None,
        status: "active",
        updated_offset: 20,
        expected: InjectionExpectation::Forbidden,
    },
    FixtureMemory {
        id: 5,
        project: PROJECT,
        topic_key: "inject-deleted-advice",
        title: "Deleted project advice",
        content: "Deleted memories must not be rendered by the injection path.",
        memory_type: "discovery",
        branch: None,
        status: "deleted",
        updated_offset: 30,
        expected: InjectionExpectation::Forbidden,
    },
    FixtureMemory {
        id: 6,
        project: PROJECT,
        topic_key: "inject-filler-telemetry",
        title: "Telemetry fixture filler",
        content: "Filler memory keeps the fixture realistic without affecting pass/fail expectations.",
        memory_type: "discovery",
        branch: None,
        status: "active",
        updated_offset: -2,
        expected: InjectionExpectation::Filler,
    },
    FixtureMemory {
        id: 7,
        project: ABSTENTION_PROJECT,
        topic_key: "inject-abstention-unrelated",
        title: ABSTENTION_FORBIDDEN_TITLE,
        content: "Legacy release checklist for cache warmup.",
        memory_type: "decision",
        branch: None,
        status: "active",
        updated_offset: 40,
        expected: InjectionExpectation::Filler,
    },
    FixtureMemory {
        id: 8,
        project: PROJECT,
        topic_key: "inject-stale-source-anchor",
        title: STALE_ANCHOR_TITLE,
        content: "The legacy source anchor references src/stale_anchor.rs and must be verified before trust after later code changes.",
        memory_type: "decision",
        branch: Some(CURRENT_BRANCH),
        status: "active",
        updated_offset: -3,
        expected: InjectionExpectation::Expected,
    },
];

pub fn run_sandbox_eval(options: InjectionEvalOptions) -> Result<InjectionEvalReport> {
    let temp_data_dir = TempDataDir::new()?;
    let data_dir = temp_data_dir.path.clone();
    let result = crate::db::core::with_data_dir(&data_dir, || {
        crate::log::with_log_dir(&data_dir, || run_sandbox_eval_inner(options, &data_dir))
    });
    cleanup_data_dir_after_eval(temp_data_dir, options.keep_data_dir, result)
}

fn run_sandbox_eval_inner(
    options: InjectionEvalOptions,
    data_dir: &Path,
) -> Result<InjectionEvalReport> {
    let mut conn = crate::db::open_db().context("open sandbox injection eval DB")?;
    seed_fixture(&mut conn).context("seed injection eval fixture")?;
    let user_prompt_context = crate::context::prompt_submit_additional_context(
        &conn,
        PROJECT,
        PROJECT,
        USER_PROMPT_SESSION,
        "How do we fix startup migration races?",
        Some(USER_PROMPT_HOST),
    )
    .context("render UserPromptSubmit matching additionalContext")?;
    let user_prompt_abstention_context = crate::context::prompt_submit_additional_context(
        &conn,
        ABSTENTION_PROJECT,
        ABSTENTION_PROJECT,
        USER_PROMPT_SESSION,
        "Investigate quantum telemetry routing",
        Some(USER_PROMPT_HOST),
    )
    .context("render UserPromptSubmit abstention additionalContext")?;
    drop(conn);

    let snapshot =
        crate::context::session_start_eval_snapshot(PROJECT, PROJECT, Some(CURRENT_BRANCH), HOST)
            .context("render SessionStart injection context")?;
    let abstention_snapshot = crate::context::session_start_eval_snapshot(
        ABSTENTION_PROJECT,
        ABSTENTION_PROJECT,
        Some(CURRENT_BRANCH),
        HOST,
    )
    .context("render abstention SessionStart injection context")?;

    let expected_cases = evaluate_cases(&snapshot.rendered_output, InjectionExpectation::Expected);
    let forbidden_cases =
        evaluate_cases(&snapshot.rendered_output, InjectionExpectation::Forbidden);
    let expected_memory_recall = InjectionRateMetric::new(
        expected_cases.iter().filter(|case| case.matched).count(),
        expected_cases.len(),
    );
    let forbidden_memory_exclusion = InjectionRateMetric::new(
        forbidden_cases.iter().filter(|case| case.matched).count(),
        forbidden_cases.len(),
    );
    let abstention_passed = !abstention_snapshot
        .rendered_output
        .contains(ABSTENTION_FORBIDDEN_TITLE);
    let abstention_false_positive_bound =
        InjectionRateMetric::new(usize::from(abstention_passed), 1);
    let stale_anchor_labeling_passed = snapshot.rendered_output.contains(STALE_ANCHOR_TITLE)
        && snapshot
            .rendered_output
            .contains("source_anchor=verify-before-trust");
    let stale_anchor_labeling =
        InjectionRateMetric::new(usize::from(stale_anchor_labeling_passed), 1);
    let user_prompt_submit_passed = user_prompt_context
        .as_deref()
        .is_some_and(|output| output.contains("Migration locking fix"));
    let user_prompt_submit_memory_recall =
        InjectionRateMetric::new(usize::from(user_prompt_submit_passed), 1);
    let user_prompt_submit_abstention_passed = user_prompt_abstention_context.is_none();
    let user_prompt_submit_abstention_false_positive_bound =
        InjectionRateMetric::new(usize::from(user_prompt_submit_abstention_passed), 1);
    let all_checks_passed = expected_memory_recall.is_perfect()
        && forbidden_memory_exclusion.is_perfect()
        && abstention_false_positive_bound.is_perfect()
        && stale_anchor_labeling.is_perfect()
        && user_prompt_submit_memory_recall.is_perfect()
        && user_prompt_submit_abstention_false_positive_bound.is_perfect();
    let mut failing_examples = Vec::new();
    for case in expected_cases.iter().filter(|case| !case.matched) {
        failing_examples.push(format!("missing expected memory: {}", case.title));
    }
    for case in forbidden_cases.iter().filter(|case| !case.matched) {
        failing_examples.push(format!("rendered forbidden memory: {}", case.title));
    }
    if !abstention_passed {
        failing_examples.push(format!(
            "abstention rendered unrelated memory: {ABSTENTION_FORBIDDEN_TITLE}"
        ));
    }
    if !stale_anchor_labeling_passed {
        failing_examples
            .push("stale source-anchor memory missing verify-before-trust label".to_string());
    }
    if !user_prompt_submit_passed {
        failing_examples
            .push("UserPromptSubmit missing expected memory: Migration locking fix".to_string());
    }
    if !user_prompt_submit_abstention_passed {
        failing_examples.push("UserPromptSubmit rendered unexpected additionalContext".to_string());
    }

    let mut cases = expected_cases;
    cases.extend(forbidden_cases);
    Ok(InjectionEvalReport {
        metadata: InjectionEvalMetadata {
            corpus: CORPUS_NAME.to_string(),
            boundary: "context::render_context_output".to_string(),
            storage: "temporary sqlite".to_string(),
            data_dir: data_dir.display().to_string(),
            data_dir_kept: options.keep_data_dir,
            real_db_touched: false,
            project: PROJECT.to_string(),
            host: HOST.to_string(),
            branch: CURRENT_BRANCH.to_string(),
            output_chars: snapshot.output_chars,
            memories_loaded: snapshot.memories_loaded,
            core_count: snapshot.core_count,
            index_count: snapshot.index_count,
            lesson_count: snapshot.lesson_count,
            preference_count: snapshot.preference_count,
            session_count: snapshot.session_count,
            workstream_count: snapshot.workstream_count,
            truncated: snapshot.truncated,
        },
        metrics: InjectionMetricSummary {
            expected_memory_recall,
            forbidden_memory_exclusion,
            abstention_false_positive_bound,
            stale_anchor_labeling,
            user_prompt_submit_memory_recall,
            user_prompt_submit_abstention_false_positive_bound,
            all_checks_passed,
        },
        cases,
        failing_examples,
    })
}

fn evaluate_cases(output: &str, expectation: InjectionExpectation) -> Vec<InjectionCaseReport> {
    FIXTURE_MEMORIES
        .iter()
        .filter(|memory| memory.expected == expectation)
        .map(|memory| {
            let present = output.contains(memory.title);
            let matched = match expectation {
                InjectionExpectation::Expected => present,
                InjectionExpectation::Forbidden => !present,
                InjectionExpectation::Filler => true,
            };
            InjectionCaseReport {
                id: memory.id.to_string(),
                expectation: expectation.as_str().to_string(),
                title: memory.title.to_string(),
                topic_key: memory.topic_key.to_string(),
                matched,
            }
        })
        .collect()
}

impl InjectionExpectation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Expected => "expected",
            Self::Forbidden => "forbidden",
            Self::Filler => "filler",
        }
    }
}

fn seed_fixture(conn: &mut Connection) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let tx = conn.transaction()?;
    for memory in FIXTURE_MEMORIES {
        tx.execute(
            "INSERT INTO memories
             (id, session_id, project, topic_key, title, content, memory_type, files,
              created_at_epoch, updated_at_epoch, status, branch, scope)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10, ?11, 'project')",
            params![
                memory.id,
                fixture_session_id(memory),
                memory.project,
                memory.topic_key,
                memory.title,
                memory.content,
                memory.memory_type,
                fixture_files(memory),
                now + memory.updated_offset,
                memory.status,
                memory.branch,
            ],
        )?;
    }
    seed_stale_anchor_commits(&tx, now)?;
    tx.execute(
        "INSERT INTO workstreams
         (id, project, title, description, status, progress, next_action, blockers,
          created_at_epoch, updated_at_epoch, completed_at_epoch)
         VALUES (1, ?1, 'Prompt-aware task with no memory match', NULL, 'active',
                 NULL, 'Investigate quantum telemetry routing', NULL, ?2, ?2, NULL)",
        params![ABSTENTION_PROJECT, now],
    )?;
    tx.commit()?;
    Ok(())
}

fn fixture_session_id(memory: &FixtureMemory) -> Option<&'static str> {
    (memory.id == 8).then_some("inject-stale-source-anchor-session")
}

fn fixture_files(memory: &FixtureMemory) -> Option<&'static str> {
    (memory.id == 8).then_some(r#"["src/stale_anchor.rs"]"#)
}

fn seed_stale_anchor_commits(tx: &rusqlite::Transaction<'_>, now: i64) -> Result<()> {
    let source_epoch = now - 20;
    let later_epoch = now - 10;
    tx.execute(
        "INSERT INTO git_commits
         (id, project, repo_path, sha, short_sha, branch, message, authored_at_epoch,
          changed_files, created_at_epoch, updated_at_epoch)
         VALUES (1, ?1, ?1, 'source-anchor-sha', 'source-', 'main',
                 'Capture stale anchor source', ?2, ?3, ?2, ?2)",
        params![
            PROJECT,
            source_epoch,
            serde_json::to_string(&["src/stale_anchor.rs"])?
        ],
    )?;
    tx.execute(
        "INSERT INTO git_commit_sessions
         (commit_id, session_id, memory_session_id, source, linked_at_epoch)
         VALUES (1, 'content-stale-anchor', 'inject-stale-source-anchor-session',
                 'test', ?1)",
        params![source_epoch],
    )?;
    tx.execute(
        "INSERT INTO git_commits
         (id, project, repo_path, sha, short_sha, branch, message, authored_at_epoch,
          changed_files, created_at_epoch, updated_at_epoch)
         VALUES (2, ?1, ?1, 'later-anchor-sha', 'later-a', 'main',
                 'Change stale anchor file', ?2, ?3, ?2, ?2)",
        params![
            PROJECT,
            later_epoch,
            serde_json::to_string(&["src/stale_anchor.rs"])?
        ],
    )?;
    Ok(())
}

fn unique_temp_data_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "remem-injection-eval-{}-{}",
        std::process::id(),
        nanos
    ))
}

struct TempDataDir {
    path: PathBuf,
    cleaned: bool,
}

impl TempDataDir {
    fn new() -> Result<Self> {
        let path = unique_temp_data_dir();
        std::fs::create_dir_all(&path)
            .with_context(|| format!("create injection eval data dir {}", path.display()))?;
        crate::db::core::with_data_dir(&path, crate::db::generate_cipher_key)
            .context("create injection eval database key")?;
        Ok(Self {
            path,
            cleaned: false,
        })
    }

    fn cleanup(&mut self) -> Result<()> {
        std::fs::remove_dir_all(&self.path)
            .with_context(|| format!("remove injection eval data dir {}", self.path.display()))?;
        self.cleaned = true;
        Ok(())
    }
}

impl Drop for TempDataDir {
    fn drop(&mut self) {
        if self.cleaned {
            return;
        }
        if let Err(cleanup_err) = std::fs::remove_dir_all(&self.path) {
            crate::log::warn(
                "eval-injection",
                &format!("cleanup failed during drop: {}", cleanup_err),
            );
        }
    }
}

fn cleanup_data_dir_after_eval<T>(
    mut temp_data_dir: TempDataDir,
    keep_data_dir: bool,
    result: Result<T>,
) -> Result<T> {
    if keep_data_dir {
        temp_data_dir.cleaned = true;
        return result;
    }
    let cleanup = temp_data_dir.cleanup();
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(err)) => Err(err),
        (Err(err), Ok(())) => Err(err),
        (Err(err), Err(cleanup_err)) => {
            crate::log::warn(
                "eval-injection",
                &format!("cleanup failed after eval error: {}", cleanup_err),
            );
            Err(err)
        }
    }
}
