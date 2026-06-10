use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::collections::BTreeSet;

use super::{evaluate_dataset, load_dataset, EvidenceRef, GoldenDataset, GoldenQuery, QueryStatus};

fn setup_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::memory::tests_helper::setup_memory_schema(&conn);
    Ok(conn)
}

struct TestMemory<'a> {
    id: i64,
    project: &'a str,
    topic_key: &'a str,
    title: &'a str,
    content: &'a str,
    memory_type: &'a str,
    branch: Option<&'a str>,
    status: &'a str,
    updated_at_epoch: i64,
}

fn insert_memory(conn: &Connection, memory: &TestMemory<'_>) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, ?10, ?11, 'project')",
        params![
            memory.id,
            format!("session-{}", memory.id),
            memory.project,
            memory.topic_key,
            memory.title,
            memory.content,
            memory.memory_type,
            memory.updated_at_epoch,
            memory.updated_at_epoch,
            memory.status,
            memory.branch,
        ],
    )?;
    Ok(())
}

fn query(
    id: &str,
    text: &str,
    category: &str,
    project: Option<&str>,
    branch: Option<&str>,
    evidence_ref: EvidenceRef,
) -> GoldenQuery {
    GoldenQuery {
        id: id.to_string(),
        query: text.to_string(),
        category: category.to_string(),
        slice: None,
        project: project.map(str::to_string),
        branch: branch.map(str::to_string),
        memory_type: None,
        relevant_ids: vec![],
        evidence_refs: vec![evidence_ref],
        expect_abstain: false,
        false_premise: false,
        notes: None,
    }
}

#[test]
fn golden_eval_scores_core_categories_and_abstention() -> Result<()> {
    let conn = setup_conn()?;
    let now = chrono::Utc::now().timestamp();
    for memory in [
        TestMemory {
            id: 1,
            project: "/repo-a",
            topic_key: "repo-a-project-scope",
            title: "SQLite project scoped fix",
            content: "SQLite WAL timeout fix belongs to repo A.",
            memory_type: "bugfix",
            branch: Some("main"),
            status: "active",
            updated_at_epoch: now - 100,
        },
        TestMemory {
            id: 2,
            project: "/repo-b",
            topic_key: "repo-b-project-scope",
            title: "SQLite project scoped fix",
            content: "SQLite WAL timeout fix belongs to repo B.",
            memory_type: "bugfix",
            branch: Some("main"),
            status: "active",
            updated_at_epoch: now - 90,
        },
        TestMemory {
            id: 3,
            project: "/repo-a",
            topic_key: "recent-temporal-fix",
            title: "Recent deploy fix",
            content: "recent deploy fix for worker heartbeat",
            memory_type: "bugfix",
            branch: Some("main"),
            status: "active",
            updated_at_epoch: now - 3_600,
        },
        TestMemory {
            id: 4,
            project: "/repo-a",
            topic_key: "port-old",
            title: "Old port decision",
            content: "Use old port 1234 for local API.",
            memory_type: "decision",
            branch: Some("main"),
            status: "archived",
            updated_at_epoch: now - 200,
        },
        TestMemory {
            id: 5,
            project: "/repo-a",
            topic_key: "port-current",
            title: "Current port decision",
            content: "Use corrected port 5567 for local API.",
            memory_type: "decision",
            branch: Some("main"),
            status: "active",
            updated_at_epoch: now - 50,
        },
        TestMemory {
            id: 6,
            project: "/repo-a",
            topic_key: "procedure-pr-review",
            title: "PR review procedure",
            content: "Run tests, post @codex review, fix feedback, then merge.",
            memory_type: "procedure",
            branch: Some("main"),
            status: "active",
            updated_at_epoch: now - 40,
        },
        TestMemory {
            id: 7,
            project: "/repo-a",
            topic_key: "branch-main",
            title: "Branch scoped note",
            content: "branch scoped needle belongs to main.",
            memory_type: "discovery",
            branch: Some("main"),
            status: "active",
            updated_at_epoch: now - 30,
        },
        TestMemory {
            id: 8,
            project: "/repo-a",
            topic_key: "branch-feature",
            title: "Branch scoped note",
            content: "branch scoped needle belongs to feature.",
            memory_type: "discovery",
            branch: Some("feature"),
            status: "active",
            updated_at_epoch: now - 20,
        },
    ] {
        insert_memory(&conn, &memory)?;
    }

    let dataset = GoldenDataset {
        version: Some("1.2-test".to_string()),
        description: Some("test fixture".to_string()),
        queries: vec![
            query(
                "project",
                "SQLite WAL timeout",
                "project_scope",
                Some("/repo-a"),
                Some("main"),
                EvidenceRef {
                    topic_key: Some("repo-a-project-scope".to_string()),
                    project: Some("/repo-a".to_string()),
                    branch: Some("main".to_string()),
                    ..EvidenceRef::default()
                },
            ),
            query(
                "temporal",
                "recent deploy fix",
                "temporal",
                Some("/repo-a"),
                Some("main"),
                EvidenceRef {
                    topic_key: Some("recent-temporal-fix".to_string()),
                    ..EvidenceRef::default()
                },
            ),
            query(
                "update",
                "corrected port 5567",
                "knowledge_update",
                Some("/repo-a"),
                Some("main"),
                EvidenceRef {
                    topic_key: Some("port-current".to_string()),
                    text_contains: Some("5567".to_string()),
                    ..EvidenceRef::default()
                },
            ),
            query(
                "procedure",
                "PR review procedure",
                "procedure",
                Some("/repo-a"),
                Some("main"),
                EvidenceRef {
                    memory_type: Some("procedure".to_string()),
                    text_contains: Some("@codex review".to_string()),
                    ..EvidenceRef::default()
                },
            ),
            query(
                "branch",
                "branch scoped needle",
                "project_scope",
                Some("/repo-a"),
                Some("main"),
                EvidenceRef {
                    topic_key: Some("branch-main".to_string()),
                    branch: Some("main".to_string()),
                    ..EvidenceRef::default()
                },
            ),
            GoldenQuery {
                id: "abstain".to_string(),
                query: "MongoDB migration nonexistent".to_string(),
                category: "abstention".to_string(),
                slice: None,
                project: Some("/repo-a".to_string()),
                branch: Some("main".to_string()),
                memory_type: None,
                relevant_ids: vec![],
                evidence_refs: vec![],
                expect_abstain: true,
                false_premise: true,
                notes: None,
            },
        ],
    };

    let report = evaluate_dataset(&conn, &dataset, 5)?;

    assert_eq!(report.scored_queries, 5);
    assert_eq!(report.abstention_queries, 1);
    assert_eq!(report.abstention_passed, 1);
    let overall = report.overall.as_ref().context("missing overall metrics")?;
    assert_eq!(overall.hit_at_k, 1.0);
    let project_scope = report
        .by_category
        .get("project_scope")
        .context("missing project_scope category")?;
    let project_scope_metrics = project_scope
        .metrics
        .as_ref()
        .context("missing project_scope metrics")?;
    assert_eq!(project_scope_metrics.count, 2);
    let project_scope_slice = report
        .by_slice
        .get("project_scope")
        .context("missing default project_scope slice")?;
    assert_eq!(project_scope_slice.scored_queries, 2);
    assert_eq!(
        report.queries.last().map(|query| query.status),
        Some(QueryStatus::Pass)
    );
    Ok(())
}

#[test]
fn golden_eval_rejects_empty_evidence_refs() -> Result<()> {
    let conn = setup_conn()?;
    let dataset = GoldenDataset {
        version: Some("1.2-test".to_string()),
        description: None,
        queries: vec![query(
            "bad",
            "bad query",
            "bad",
            None,
            None,
            EvidenceRef::default(),
        )],
    };

    let Err(error) = evaluate_dataset(&conn, &dataset, 5) else {
        panic!("empty evidence ref should fail validation");
    };

    assert!(error.to_string().contains("empty evidence ref"));
    Ok(())
}

#[test]
fn golden_eval_miss_records_retrieved_and_missing_ids() -> Result<()> {
    let conn = setup_conn()?;
    let now = chrono::Utc::now().timestamp();
    insert_memory(
        &conn,
        &TestMemory {
            id: 1,
            project: "/repo-a",
            topic_key: "wrong-eval-result",
            title: "Wrong eval result",
            content: "wrong eval needle is retrievable but irrelevant",
            memory_type: "discovery",
            branch: Some("main"),
            status: "active",
            updated_at_epoch: now,
        },
    )?;

    let dataset = GoldenDataset {
        version: Some("1.2-test".to_string()),
        description: None,
        queries: vec![GoldenQuery {
            id: "miss".to_string(),
            query: "wrong eval needle".to_string(),
            category: "single_hop".to_string(),
            slice: None,
            project: Some("/repo-a".to_string()),
            branch: Some("main".to_string()),
            memory_type: None,
            relevant_ids: vec![42],
            evidence_refs: vec![],
            expect_abstain: false,
            false_premise: false,
            notes: None,
        }],
    };

    let report = evaluate_dataset(&conn, &dataset, 5)?;
    let query = &report.queries[0];

    assert_eq!(query.status, QueryStatus::Miss);
    assert_eq!(query.retrieved_ids, vec![1]);
    assert_eq!(query.expected_relevant_ids, vec![42]);
    assert_eq!(query.missing_relevant_ids, vec![42]);
    assert_eq!(query.missing_evidence_refs.len(), 1);

    let rendered = report.to_string();
    assert!(rendered.contains("--- Evaluation Layers ---"));
    assert!(rendered.contains("--- Retrieval Overall"));
    assert!(rendered.contains("--- Answer/Judge Layer ---"));
    assert!(rendered.contains("retrieved_ids=[1]"));
    assert!(rendered.contains("missing_relevant_ids=[42]"));

    let json = serde_json::to_value(&report)?;
    assert_eq!(
        json["evaluation_layers"]["retrieval"]["status"],
        "deterministic"
    );
    assert_eq!(
        json["evaluation_layers"]["answer_generation"]["status"],
        "not_run"
    );
    assert_eq!(json["queries"][0]["status"], "MISS");
    assert_eq!(json["queries"][0]["slice"], "single_hop");
    assert_eq!(json["by_slice"]["single_hop"]["metrics"]["count"], 1);
    assert_eq!(json["queries"][0]["retrieved_ids"][0], 1);
    assert_eq!(json["queries"][0]["missing_relevant_ids"][0], 42);
    Ok(())
}

#[test]
fn golden_eval_ndcg_counts_each_expected_ref_once() -> Result<()> {
    let conn = setup_conn()?;
    let now = chrono::Utc::now().timestamp();
    for memory in [
        TestMemory {
            id: 1,
            project: "/repo-a",
            topic_key: "dup-a",
            title: "Duplicate eval needle",
            content: "duplicate ndcg needle from one memory",
            memory_type: "bugfix",
            branch: Some("main"),
            status: "active",
            updated_at_epoch: now,
        },
        TestMemory {
            id: 2,
            project: "/repo-a",
            topic_key: "dup-b",
            title: "Duplicate eval needle",
            content: "duplicate ndcg needle from another memory",
            memory_type: "bugfix",
            branch: Some("main"),
            status: "active",
            updated_at_epoch: now - 1,
        },
    ] {
        insert_memory(&conn, &memory)?;
    }

    let dataset = GoldenDataset {
        version: Some("1.2-test".to_string()),
        description: None,
        queries: vec![query(
            "dup",
            "duplicate ndcg needle",
            "dedupe",
            Some("/repo-a"),
            Some("main"),
            EvidenceRef {
                text_contains: Some("duplicate ndcg needle".to_string()),
                ..EvidenceRef::default()
            },
        )],
    };

    let report = evaluate_dataset(&conn, &dataset, 10)?;
    let metrics = report.queries[0]
        .metrics
        .as_ref()
        .context("missing query metrics")?;
    assert_eq!(report.queries[0].matched_refs, 1);
    assert!(metrics.ndcg_at_10 <= 1.0, "{metrics:?}");
    assert_eq!(metrics.ndcg_at_10, 1.0);
    Ok(())
}

#[test]
fn golden_eval_ndcg_uses_best_assignment_for_overlapping_refs() -> Result<()> {
    let conn = setup_conn()?;
    let now = chrono::Utc::now().timestamp();
    for memory in [
        TestMemory {
            id: 1,
            project: "/repo-a",
            topic_key: "specific-overlap",
            title: "Overlapping assignment needle",
            content: "overlapping assignment shared unique specific memory",
            memory_type: "bugfix",
            branch: Some("main"),
            status: "active",
            updated_at_epoch: now,
        },
        TestMemory {
            id: 2,
            project: "/repo-a",
            topic_key: "broad-only",
            title: "Overlapping assignment needle",
            content: "overlapping assignment shared broad memory",
            memory_type: "bugfix",
            branch: Some("main"),
            status: "active",
            updated_at_epoch: now - 1,
        },
    ] {
        insert_memory(&conn, &memory)?;
    }

    let dataset = GoldenDataset {
        version: Some("1.2-test".to_string()),
        description: None,
        queries: vec![GoldenQuery {
            id: "overlap".to_string(),
            query: "overlapping assignment shared".to_string(),
            category: "dedupe".to_string(),
            slice: Some("multi_hop".to_string()),
            project: Some("/repo-a".to_string()),
            branch: Some("main".to_string()),
            memory_type: None,
            relevant_ids: vec![],
            evidence_refs: vec![
                EvidenceRef {
                    text_contains: Some("overlapping assignment shared".to_string()),
                    ..EvidenceRef::default()
                },
                EvidenceRef {
                    topic_key: Some("specific-overlap".to_string()),
                    ..EvidenceRef::default()
                },
            ],
            expect_abstain: false,
            false_premise: false,
            notes: None,
        }],
    };

    let report = evaluate_dataset(&conn, &dataset, 10)?;
    let metrics = report.queries[0]
        .metrics
        .as_ref()
        .context("missing query metrics")?;
    assert_eq!(report.queries[0].matched_refs, 2);
    assert_eq!(report.queries[0].slice, "multi_hop");
    assert_eq!(
        report
            .by_slice
            .get("multi_hop")
            .context("missing multi_hop slice")?
            .scored_queries,
        1
    );
    assert_eq!(metrics.ndcg_at_10, 1.0);
    Ok(())
}

#[test]
fn checked_in_golden_dataset_has_required_slices() -> Result<()> {
    let dataset = load_dataset("eval/golden.json")?;
    assert!(
        dataset.queries.len() >= 50,
        "golden dataset should have at least 50 cases, got {}",
        dataset.queries.len()
    );

    let slices: BTreeSet<_> = dataset
        .queries
        .iter()
        .map(|query| query.slice_label().to_string())
        .collect();
    for required in [
        "paraphrase",
        "knowledge_update",
        "temporal",
        "abstention",
        "multi_hop",
    ] {
        assert!(slices.contains(required), "missing slice {required}");
    }

    for query in dataset
        .queries
        .iter()
        .filter(|query| query.slice_label() == "abstention")
    {
        assert!(
            query.expects_abstention(),
            "abstention slice query {} must assert abstention",
            query.id
        );
        assert!(
            query.evidence_refs.is_empty() && query.relevant_ids.is_empty(),
            "abstention query {} must not declare evidence",
            query.id
        );
    }
    Ok(())
}

#[test]
fn golden_eval_records_current_paraphrase_baseline_for_issue_358() -> Result<()> {
    let conn = setup_conn()?;
    insert_memory(
        &conn,
        &TestMemory {
            id: 1,
            project: "/repo-a",
            topic_key: "glimmerdock-ledger",
            title: "Glimmerdock ledger",
            content: "Mira Vale controls violet rail allowance.",
            memory_type: "decision",
            branch: Some("main"),
            status: "active",
            updated_at_epoch: chrono::Utc::now().timestamp(),
        },
    )?;

    let dataset = GoldenDataset {
        version: Some("1.2-test".to_string()),
        description: None,
        queries: vec![GoldenQuery {
            id: "paraphrase-baseline-358".to_string(),
            query: "owner mauve locomotive ration".to_string(),
            category: "retrieval".to_string(),
            slice: Some("paraphrase".to_string()),
            project: Some("/repo-a".to_string()),
            branch: Some("main".to_string()),
            memory_type: None,
            relevant_ids: vec![],
            evidence_refs: vec![EvidenceRef {
                topic_key: Some("glimmerdock-ledger".to_string()),
                text_contains: Some("violet rail allowance".to_string()),
                ..EvidenceRef::default()
            }],
            expect_abstain: false,
            false_premise: false,
            notes: Some("Current feature-hash retrieval baseline for #358.".to_string()),
        }],
    };

    let report = evaluate_dataset(&conn, &dataset, 5)?;
    let metrics = report.queries[0]
        .metrics
        .as_ref()
        .context("missing paraphrase metrics")?;
    assert_eq!(report.queries[0].status, QueryStatus::Miss);
    assert_eq!(metrics.hit_at_k, 0.0);
    assert_eq!(metrics.mrr_at_10, 0.0);
    assert_eq!(metrics.recall_at_k, 0.0);
    let paraphrase = report
        .by_slice
        .get("paraphrase")
        .context("missing paraphrase slice")?
        .metrics
        .as_ref()
        .context("missing paraphrase slice metrics")?;
    assert_eq!(paraphrase.count, 1);
    assert_eq!(paraphrase.hit_at_k, 0.0);
    assert_eq!(paraphrase.mrr_at_10, 0.0);
    Ok(())
}
