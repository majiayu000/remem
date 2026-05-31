use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db::{self, CaptureEventInput};
use crate::memory::{self, Memory};

use super::types::{LifecycleCounts, OwnerCheckReport};

pub(super) const CORPUS_NAME: &str = "builtin-memory-governance-v1";
pub(super) const PROJECT: &str = "/tmp/remem-governance/repo";
pub(super) const NESTED_SRC_PROJECT: &str = "/tmp/remem-governance/repo/src";
pub(super) const NESTED_CRATE_PROJECT: &str = "/tmp/remem-governance/repo/crates/agent";
pub(super) const EXPECTED_SUMMARY_CANDIDATES: usize = 7;
const SUMMARY_SESSION: &str = "governance-summary-session";

#[derive(Clone, Copy)]
pub(super) struct ExpectedOwner {
    topic_key: &'static str,
    owner_scope: &'static str,
    owner_key: &'static str,
    target_project: Option<&'static str>,
}

pub(super) struct ExpectedCandidateOwner {
    text_contains: &'static str,
    owner_scope: &'static str,
    owner_key: &'static str,
    target_project: Option<&'static str>,
}

#[derive(Clone, Copy)]
pub(super) struct SearchScenario {
    pub id: &'static str,
    pub category: &'static str,
    pub query: &'static str,
    pub project: &'static str,
    pub memory_type: Option<&'static str>,
    pub branch: Option<&'static str>,
    pub expected_topic_keys: &'static [&'static str],
    pub allowed_topic_keys: &'static [&'static str],
    pub forbidden_title_contains: &'static [&'static str],
}

pub(super) const SEARCH_SCENARIOS: &[SearchScenario] = &[
    SearchScenario {
        id: "nested-repo-cwd",
        category: "owner_retrieval",
        query: "drag drop pointer sensors",
        project: NESTED_SRC_PROJECT,
        memory_type: None,
        branch: None,
        expected_topic_keys: &["repo-dnd-pointer"],
        allowed_topic_keys: &["repo-dnd-pointer"],
        forbidden_title_contains: &[],
    },
    SearchScenario {
        id: "repo-mentioning-codex",
        category: "owner_retrieval",
        query: "approval_panel.rs",
        project: PROJECT,
        memory_type: None,
        branch: Some("main"),
        expected_topic_keys: &["repo-codex-file-exception"],
        allowed_topic_keys: &["repo-codex-file-exception"],
        forbidden_title_contains: &["Codex approval mode"],
    },
    SearchScenario {
        id: "updated-current-fact",
        category: "active_current",
        query: "emerald toolbar theme",
        project: PROJECT,
        memory_type: None,
        branch: Some("main"),
        expected_topic_keys: &["repo-toolbar-color"],
        allowed_topic_keys: &["repo-toolbar-color"],
        forbidden_title_contains: &["Toolbar color was blue"],
    },
    SearchScenario {
        id: "false-premise-stale-state",
        category: "stale_exclusion",
        query: "blue v1",
        project: PROJECT,
        memory_type: None,
        branch: Some("main"),
        expected_topic_keys: &[],
        allowed_topic_keys: &[],
        forbidden_title_contains: &["Toolbar color was blue"],
    },
    SearchScenario {
        id: "branch-mismatch",
        category: "stale_exclusion",
        query: "wasm snapshot",
        project: PROJECT,
        memory_type: None,
        branch: Some("main"),
        expected_topic_keys: &[],
        allowed_topic_keys: &[],
        forbidden_title_contains: &["Feature branch wasm snapshot"],
    },
    SearchScenario {
        id: "branch-current",
        category: "active_current",
        query: "main branch build cache policy",
        project: PROJECT,
        memory_type: Some("decision"),
        branch: Some("main"),
        expected_topic_keys: &["decision-1111111111111111"],
        allowed_topic_keys: &["decision-1111111111111111"],
        forbidden_title_contains: &["Feature branch wasm snapshot"],
    },
];

pub(super) struct FixtureSeed {
    pub expected_owners: Vec<ExpectedOwner>,
    pub expected_candidate_owners: Vec<ExpectedCandidateOwner>,
    pub lifecycle_counts: LifecycleCounts,
}

pub(super) fn seed_fixture(conn: &mut Connection) -> Result<FixtureSeed> {
    let mut expected_owners = Vec::new();
    let mut lifecycle_counts = LifecycleCounts::default();

    insert_repo_fixtures(conn, &mut expected_owners)?;
    insert_pollution_fixtures(conn, &mut expected_owners)?;
    insert_lifecycle_fixtures(conn, &mut expected_owners, &mut lifecycle_counts)?;
    insert_branch_fixtures(conn, &mut expected_owners)?;
    let mut expected_candidate_owners = Vec::new();
    record_summary_evidence(conn, SUMMARY_SESSION, "Generic summary evidence")?;
    let summary_count = memory::promote_summary_to_memory_candidates(
        conn,
        SUMMARY_SESSION,
        PROJECT,
        Some("Build realistic memory governance eval"),
        Some("Use an in-memory governance eval suite for owner and lifecycle quality checks."),
        Some("Lesson: summary-derived durable facts must become candidates before activation."),
        Some("Review summary-derived preferences before activation."),
    )?;
    if summary_count == 0 {
        anyhow::bail!("summary promotion did not create governance eval candidates");
    }
    expected_candidate_owners.extend([
        ExpectedCandidateOwner {
            text_contains: "in-memory governance eval suite",
            owner_scope: "repo",
            owner_key: PROJECT,
            target_project: Some(PROJECT),
        },
        ExpectedCandidateOwner {
            text_contains: "summary-derived durable facts",
            owner_scope: "repo",
            owner_key: PROJECT,
            target_project: Some(PROJECT),
        },
        ExpectedCandidateOwner {
            text_contains: "Review summary-derived preferences",
            owner_scope: "repo",
            owner_key: PROJECT,
            target_project: Some(PROJECT),
        },
    ]);
    seed_routing_candidate_fixtures(conn, &mut expected_candidate_owners)?;

    Ok(FixtureSeed {
        expected_owners,
        expected_candidate_owners,
        lifecycle_counts,
    })
}

fn seed_routing_candidate_fixtures(
    conn: &mut Connection,
    expected_candidate_owners: &mut Vec<ExpectedCandidateOwner>,
) -> Result<()> {
    for (session, text, owner_scope, owner_key, target_project) in [
        (
            "governance-route-repo-codex",
            "The repo file src/approval_panel.rs renders Codex approval copy for product UI.",
            "repo",
            PROJECT,
            Some(PROJECT),
        ),
        (
            "governance-route-tool-codex",
            "Codex CLI sandbox approval mode must stay workspace-write.",
            "tool",
            "codex-cli",
            None,
        ),
        (
            "governance-route-domain-grok",
            "Grok API accepts image references for xAI integration tests.",
            "domain",
            "grok-api",
            None,
        ),
        (
            "governance-route-domain-warp",
            "Warp terminal launch configuration belongs to the macOS domain.",
            "domain",
            "macos",
            None,
        ),
    ] {
        record_summary_evidence(conn, session, text)?;
        let count = memory::promote_summary_to_memory_candidates(
            conn,
            session,
            PROJECT,
            Some("Route governance candidate"),
            Some(text),
            None,
            None,
        )?;
        if count != 1 {
            anyhow::bail!("expected one routed summary candidate for {session}, got {count}");
        }
        expected_candidate_owners.push(ExpectedCandidateOwner {
            text_contains: text,
            owner_scope,
            owner_key,
            target_project,
        });
    }
    Ok(())
}

fn insert_repo_fixtures(conn: &Connection, expected_owners: &mut Vec<ExpectedOwner>) -> Result<()> {
    for (project, topic_key, title, content, memory_type) in [
        (
            NESTED_SRC_PROJECT,
            "repo-dnd-pointer",
            "Stash DnD uses pointer sensors",
            "The nested repo UI in src/dnd.rs uses pointer sensors for drag drop ordering.",
            "decision",
        ),
        (
            NESTED_CRATE_PROJECT,
            "repo-agent-crate-cache",
            "Agent crate cache policy",
            "The crates/agent package stores eval cache files under target/remem-governance.",
            "architecture",
        ),
        (
            PROJECT,
            "repo-codex-file-exception",
            "Repo approval panel mentions Codex",
            "The repo file src/approval_panel.rs renders Codex approval copy for this product UI.",
            "decision",
        ),
    ] {
        insert_fixture_memory(
            conn,
            project,
            topic_key,
            title,
            content,
            memory_type,
            Some("main"),
            "repo",
            PROJECT,
            Some(PROJECT),
            "startup_core",
            "active",
        )?;
        expected_owners.push(ExpectedOwner {
            topic_key,
            owner_scope: "repo",
            owner_key: PROJECT,
            target_project: Some(PROJECT),
        });
    }
    Ok(())
}

fn insert_pollution_fixtures(
    conn: &Connection,
    expected_owners: &mut Vec<ExpectedOwner>,
) -> Result<()> {
    for (topic_key, title, content, owner_scope, owner_key) in [
        (
            "tool-codex-approval",
            "Codex approval mode",
            "Codex CLI approval mode uses workspace-write for sandboxed shell commands.",
            "tool",
            "codex-cli",
        ),
        (
            "domain-grok-api",
            "Grok API image references",
            "The Grok API accepts image references and belongs to the xAI integration domain.",
            "domain",
            "grok-api",
        ),
        (
            "domain-warp-config",
            "Warp terminal launch config",
            "Warp terminal launch configuration belongs to the macOS terminal domain.",
            "domain",
            "macos",
        ),
    ] {
        insert_fixture_memory(
            conn,
            PROJECT,
            topic_key,
            title,
            content,
            "discovery",
            None,
            owner_scope,
            owner_key,
            None,
            "search_only",
            "active",
        )?;
        expected_owners.push(ExpectedOwner {
            topic_key,
            owner_scope,
            owner_key,
            target_project: None,
        });
    }
    Ok(())
}

fn insert_lifecycle_fixtures(
    conn: &Connection,
    expected_owners: &mut Vec<ExpectedOwner>,
    lifecycle_counts: &mut LifecycleCounts,
) -> Result<()> {
    let add = memory::lifecycle::apply_add(
        conn,
        Some("governance-add"),
        PROJECT,
        Some("repo-test-command"),
        "Governance eval test command",
        "Run cargo test eval::governance after changing governance fixtures.",
        "decision",
        Some("src/eval/governance.rs"),
        Some("main"),
        "project",
    )?;
    count_lifecycle(lifecycle_counts, add.op);
    expected_owners.push(ExpectedOwner {
        topic_key: "repo-test-command",
        owner_scope: "repo",
        owner_key: PROJECT,
        target_project: Some(PROJECT),
    });

    let old_toolbar = insert_fixture_memory(
        conn,
        PROJECT,
        "repo-toolbar-color",
        "Toolbar color was blue",
        "Toolbar color was blue in theme v1.",
        "decision",
        Some("main"),
        "repo",
        PROJECT,
        Some(PROJECT),
        "startup_core",
        "active",
    )?;
    let update = memory::lifecycle::apply_update(
        conn,
        Some("governance-update"),
        PROJECT,
        "repo-toolbar-color",
        "Toolbar color is emerald",
        "Toolbar color is emerald in theme v2.",
        "decision",
        Some("src/theme.rs"),
        Some("main"),
        "project",
        &[old_toolbar],
    )?;
    count_lifecycle(lifecycle_counts, update.op);
    expected_owners.push(ExpectedOwner {
        topic_key: "repo-toolbar-color",
        owner_scope: "repo",
        owner_key: PROJECT,
        target_project: Some(PROJECT),
    });

    let obsolete = insert_fixture_memory(
        conn,
        PROJECT,
        "repo-dev-server-port",
        "Dev server port 3000",
        "Dev server currently running on port 3000.",
        "discovery",
        Some("main"),
        "repo",
        PROJECT,
        Some(PROJECT),
        "startup_core",
        "active",
    )?;
    let invalidate = memory::lifecycle::apply_invalidate(
        conn,
        PROJECT,
        &[obsolete],
        Some("port changed after later evidence"),
    )?;
    count_lifecycle(lifecycle_counts, invalidate.op);
    count_lifecycle(
        lifecycle_counts,
        memory::lifecycle::noop("duplicate same evidence").op,
    );
    count_lifecycle(
        lifecycle_counts,
        memory::lifecycle::defer("ambiguous owner route").op,
    );
    Ok(())
}

fn insert_branch_fixtures(
    conn: &Connection,
    expected_owners: &mut Vec<ExpectedOwner>,
) -> Result<()> {
    for (topic_key, title, content, branch) in [
        (
            "decision-1111111111111111",
            "Main branch build cache policy",
            "[Context: Governance branch cache policy]\nMain branch uses cargo test --workspace for build cache verification.",
            "main",
        ),
        (
            "decision-2222222222222222",
            "Feature branch wasm snapshot",
            "[Context: Governance branch cache policy]\nFeature branch uses an experimental wasm snapshot build cache.",
            "feature/wasm-cache",
        ),
    ] {
        insert_fixture_memory(
            conn,
            PROJECT,
            topic_key,
            title,
            content,
            "decision",
            Some(branch),
            "repo",
            PROJECT,
            Some(PROJECT),
            "startup_core",
            "active",
        )?;
        expected_owners.push(ExpectedOwner {
            topic_key,
            owner_scope: "repo",
            owner_key: PROJECT,
            target_project: Some(PROJECT),
        });
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_fixture_memory(
    conn: &Connection,
    project: &str,
    topic_key: &str,
    title: &str,
    content: &str,
    memory_type: &str,
    branch: Option<&str>,
    owner_scope: &str,
    owner_key: &str,
    target_project: Option<&str>,
    context_class: &str,
    status: &str,
) -> Result<i64> {
    let id = memory::insert_memory_full(
        conn,
        Some("governance-eval"),
        project,
        Some(topic_key),
        title,
        content,
        memory_type,
        None,
        branch,
        "project",
        None,
    )?;
    conn.execute(
        "UPDATE memories
         SET source_project = ?1,
             target_project = ?2,
             owner_scope = ?3,
             owner_key = ?4,
             routing_confidence = 1.0,
             routing_reason = 'governance eval fixture',
             context_class = ?5,
             status = ?6
         WHERE id = ?7",
        params![
            project,
            target_project,
            owner_scope,
            owner_key,
            context_class,
            status,
            id
        ],
    )?;
    Ok(id)
}

fn record_summary_evidence(conn: &Connection, session_id: &str, content: &str) -> Result<i64> {
    let outcome = db::record_captured_event(
        conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id,
            project: PROJECT,
            cwd: None,
            event_type: "session_stop",
            role: Some("assistant"),
            tool_name: None,
            content,
            task_kind: None,
        },
    )?;
    Ok(outcome.event_row_id)
}

fn count_lifecycle(counts: &mut LifecycleCounts, op: memory::lifecycle::MemoryLifecycleOp) {
    match op {
        memory::lifecycle::MemoryLifecycleOp::Add => counts.add += 1,
        memory::lifecycle::MemoryLifecycleOp::Update => counts.update += 1,
        memory::lifecycle::MemoryLifecycleOp::Invalidate => counts.invalidate += 1,
        memory::lifecycle::MemoryLifecycleOp::Noop => counts.noop += 1,
        memory::lifecycle::MemoryLifecycleOp::Defer => counts.defer += 1,
    }
}

pub(super) fn memory_owner_checks(
    conn: &Connection,
    expected: &[ExpectedOwner],
) -> Result<Vec<OwnerCheckReport>> {
    expected
        .iter()
        .map(|expected| {
            let actual = conn
                .query_row(
                    "SELECT owner_scope, owner_key, target_project
                     FROM memories
                     WHERE topic_key = ?1 AND status = 'active'
                     ORDER BY id DESC
                     LIMIT 1",
                    params![expected.topic_key],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .optional()?
                .with_context(|| format!("missing active fixture memory {}", expected.topic_key))?;
            Ok(owner_check(
                format!("memory:{}", expected.topic_key),
                expected.owner_scope,
                expected.owner_key,
                expected.target_project,
                actual,
            ))
        })
        .collect()
}

pub(super) fn summary_candidate_owner_checks(
    conn: &Connection,
    expected: &[ExpectedCandidateOwner],
) -> Result<Vec<OwnerCheckReport>> {
    let mut checks = Vec::new();
    for expected in expected {
        let pattern = format!("%{}%", expected.text_contains);
        let (id, memory_type, scope, key, target) = conn
            .query_row(
                "SELECT id, memory_type, owner_scope, owner_key, target_project
                 FROM memory_candidates
                 WHERE text LIKE ?1
                 ORDER BY id ASC
                 LIMIT 1",
                params![pattern],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()?
            .with_context(|| {
                format!(
                    "missing candidate containing text {:?}",
                    expected.text_contains
                )
            })?;
        checks.push(owner_check(
            format!("candidate:{id}:{memory_type}"),
            expected.owner_scope,
            expected.owner_key,
            expected.target_project,
            (scope, key, target),
        ));
    }
    Ok(checks)
}

fn owner_check(
    object_ref: String,
    expected_scope: &str,
    expected_key: &str,
    expected_target_project: Option<&str>,
    actual: (Option<String>, Option<String>, Option<String>),
) -> OwnerCheckReport {
    let (actual_scope, actual_key, actual_target_project) = actual;
    let pass = actual_scope.as_deref() == Some(expected_scope)
        && actual_key.as_deref() == Some(expected_key)
        && actual_target_project.as_deref() == expected_target_project;
    OwnerCheckReport {
        object_ref,
        expected_scope: expected_scope.to_string(),
        expected_key: expected_key.to_string(),
        expected_target_project: expected_target_project.map(str::to_string),
        actual_scope,
        actual_key,
        actual_target_project,
        pass,
    }
}

pub(super) fn forbidden_hits(results: &[Memory], forbidden_title_contains: &[&str]) -> Vec<String> {
    results
        .iter()
        .filter(|memory| {
            forbidden_title_contains.iter().any(|needle| {
                memory
                    .title
                    .to_ascii_lowercase()
                    .contains(&needle.to_ascii_lowercase())
            })
        })
        .map(|memory| {
            format!(
                "{}:{}",
                memory.topic_key.as_deref().unwrap_or("<no-topic>"),
                memory.title
            )
        })
        .collect()
}
