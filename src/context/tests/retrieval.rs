use anyhow::Context;
use rusqlite::{params, Connection};

use super::super::hybrid_context::query_hybrid_context_memories;
use super::super::policy::{ContextLimits, ContextPolicy};
use super::super::query::load_context_data_with_policy;
use super::super::sections::render_core_memory_with_limits;
use super::{insert_global_memory, insert_memory, setup_context_schema};

#[test]
fn load_context_data_uses_hybrid_retrieval_from_workstream_signal() {
    let conn = Connection::open_in_memory().unwrap();
    setup_context_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();
    let limits = ContextLimits {
        candidate_fetch_limit: 3,
        memory_index_limit: 10,
        core_item_limit: 4,
        ..ContextLimits::default()
    };
    let policy = ContextPolicy::from_limits(limits);

    for idx in 0..20 {
        insert_memory(
            &conn,
            idx + 1,
            project,
            Some(&format!("recent-noise-{idx}")),
            "discovery",
            &format!("Recent unrelated note {idx}"),
            "Recent context entry without the task terms.",
            now - idx,
        );
    }
    insert_memory(
        &conn,
        200,
        project,
        Some("sqlcipher-storage-decision"),
        "decision",
        "SQLCipher storage decision",
        "Persist private data with SQLCipher encryption at rest.",
        now - 10_000,
    );
    conn.execute(
        "INSERT INTO workstreams
         (id, project, title, status, next_action, created_at_epoch, updated_at_epoch)
         VALUES (1, ?1, 'Private persistence', 'active',
                 'Fix SQLCipher recall for private persisted data', ?2, ?2)",
        params![project, now],
    )
    .unwrap();

    let loaded = load_context_data_with_policy(&conn, project, None, &policy, true);

    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "SQLCipher storage decision"));
}

#[test]
fn hybrid_context_retrieval_still_excludes_global_non_preferences() {
    let conn = Connection::open_in_memory().unwrap();
    setup_context_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();
    let limits = ContextLimits {
        candidate_fetch_limit: 1,
        memory_index_limit: 10,
        core_item_limit: 3,
        ..ContextLimits::default()
    };
    let policy = ContextPolicy::from_limits(limits);

    insert_memory(
        &conn,
        1,
        project,
        Some("local-sqlcipher-decision"),
        "decision",
        "Local SQLCipher decision",
        "Repository-local SQLCipher storage decision.",
        now - 100,
    );
    insert_memory(
        &conn,
        2,
        "global",
        Some("global-sqlcipher-decision"),
        "bugfix",
        "Global SQLCipher note",
        "Global SQLCipher note should not enter project startup context.",
        now,
    );
    conn.execute(
        "UPDATE memories SET scope = 'global', owner_scope = 'user', owner_key = 'manual'
         WHERE id = 2",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO workstreams
         (id, project, title, status, next_action, created_at_epoch, updated_at_epoch)
         VALUES (1, ?1, 'SQLCipher recall', 'active',
                 'Find SQLCipher startup context decision', ?2, ?2)",
        params![project, now],
    )
    .unwrap();

    let loaded = load_context_data_with_policy(&conn, project, None, &policy, true);

    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Local SQLCipher decision"));
    assert!(!loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Global SQLCipher note"));
}

#[test]
fn hybrid_context_temporal_retrieval_uses_reference_time() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_context_schema(&conn);
    let project = "/tmp/remem";
    let wall_clock_epoch = chrono::Utc::now().timestamp();
    let event_epoch = 1_600_000_000_i64;
    insert_memory(
        &conn,
        1,
        project,
        Some("historical-reference-time"),
        "decision",
        "Historical reference time",
        "Episode provenance was captured with a historical event time.",
        wall_clock_epoch,
    );
    conn.execute(
        "UPDATE memories
         SET reference_time_epoch = ?1
         WHERE id = 1",
        params![event_epoch],
    )?;

    let memories =
        query_hybrid_context_memories(&conn, project, "what happened on 2020-09-13", None, &[], 5)?;

    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].title, "Historical reference time");
    Ok(())
}

#[test]
fn hybrid_context_fact_retrieval_labels_validity_in_core_output() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();
    let valid_from = chrono::NaiveDate::from_ymd_opt(2026, 1, 2)
        .and_then(|date| date.and_hms_opt(12, 0, 0))
        .context("valid fact label date")?
        .and_utc()
        .timestamp();
    let limits = ContextLimits {
        candidate_fetch_limit: 2,
        memory_index_limit: 4,
        core_item_limit: 4,
        core_char_limit: 1_200,
        ..ContextLimits::default()
    };
    let policy = ContextPolicy::from_limits(limits);

    for idx in 0..8 {
        insert_memory(
            &conn,
            idx + 1,
            project,
            Some(&format!("recent-noise-{idx}")),
            "session_activity",
            &format!("Recent unrelated note {idx}"),
            "Recent context entry without the fact terms.",
            now - idx,
        );
    }
    insert_memory(
        &conn,
        100,
        project,
        Some("harbormint-signer-source"),
        "decision",
        "HarborMint signer source",
        "Signer details live in the temporal fact layer. This memory body is intentionally long enough to exceed the core preview limit before the validity window would appear if the fact label were appended after the body. The rendered context must show temporal facts first so current fact validity is visible.",
        now - 10_000,
    );
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
          confidence, supersedes_fact_id, status, invalidated_at_epoch,
          created_at_epoch, updated_at_epoch)
         VALUES (?1, 'HarborMint', 'verified_by', 'Toma Reed', ?2, NULL, ?3, 100,
                 NULL, '[]', 0.95, NULL, 'active', NULL, ?3, ?3)",
        params![project, valid_from, now - 9_000],
    )?;
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
          confidence, supersedes_fact_id, status, invalidated_at_epoch,
          created_at_epoch, updated_at_epoch)
         VALUES
           (?1, 'UnrelatedService', 'verified_by', 'Mira Lane', ?2, NULL, ?3, 100,
            NULL, '[]', 0.95, NULL, 'active', NULL, ?3, ?3),
           (?1, 'UnrelatedService', 'blocked_by', 'North Region', ?2, NULL, ?3, 100,
            NULL, '[]', 0.95, NULL, 'active', NULL, ?3, ?3)",
        params![project, now - 500, now - 400],
    )?;
    conn.execute(
        "INSERT INTO workstreams
         (id, project, title, status, next_action, created_at_epoch, updated_at_epoch)
         VALUES (1, ?1, 'HarborMint signer', 'active',
                 'Who signs HarborMint with Toma Reed?', ?2, ?2)",
        params![project, now],
    )?;

    let loaded = load_context_data_with_policy(&conn, project, None, &policy, true);
    let memory = loaded
        .memories
        .iter()
        .find(|memory| memory.id == 100)
        .context("fact channel should retrieve source memory")?;

    assert!(memory.text.contains("Temporal facts:"));
    assert!(memory.text.contains("Toma Reed"));
    assert!(memory.text.contains("valid_from=2026-01-02"));
    assert!(memory.text.contains("valid_to=open"));

    let mut output = String::new();
    render_core_memory_with_limits(&mut output, &loaded.memories, &limits);
    assert!(output.contains("HarborMint signer source"), "{output}");
    assert!(output.contains("Temporal facts:"), "{output}");
    assert!(output.contains("valid_from=2026-01-02"), "{output}");
    Ok(())
}

#[test]
fn hybrid_context_fact_retrieval_filters_excluded_types_before_ranking() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();
    for (id, memory_type, title, age) in [
        (1, "preference", "Preference fact source", 100),
        (2, "lesson", "Lesson fact source", 200),
        (3, "decision", "Decision fact source", 300),
    ] {
        insert_memory(
            &conn,
            id,
            project,
            Some(title),
            memory_type,
            title,
            "Opaque source body without signer terms.",
            now - age,
        );
        conn.execute(
            "INSERT INTO memory_facts
             (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
              learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
              confidence, supersedes_fact_id, status, invalidated_at_epoch,
              created_at_epoch, updated_at_epoch)
             VALUES (?1, 'HarborMint', 'verified_by', 'Toma Reed', ?2, NULL, ?3, ?4,
                     NULL, '[]', 0.95, NULL, 'active', NULL, ?3, ?3)",
            params![project, now - 1_000, now - age, id],
        )?;
    }

    let memories = query_hybrid_context_memories(
        &conn,
        project,
        "Who signs HarborMint with Toma Reed?",
        None,
        &["preference", "lesson"],
        1,
    )?;

    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].title, "Decision fact source");
    Ok(())
}

#[test]
fn hybrid_context_fact_retrieval_applies_owner_filter_before_ranking() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();
    for id in 1..=20 {
        insert_memory(
            &conn,
            id,
            project,
            Some(&format!("foreign-owner-fact-{id}")),
            "decision",
            &format!("Foreign owner fact {id}"),
            "Opaque source body without signer terms.",
            now - id,
        );
        conn.execute(
            "UPDATE memories SET owner_scope = 'repo', owner_key = '/tmp/other' WHERE id = ?1",
            params![id],
        )?;
        conn.execute(
            "INSERT INTO memory_facts
             (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
              learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
              confidence, supersedes_fact_id, status, invalidated_at_epoch,
              created_at_epoch, updated_at_epoch)
             VALUES (?1, 'HarborMint', 'verified_by', 'Toma Reed', ?2, NULL, ?3, ?4,
                     NULL, '[]', 0.95, NULL, 'active', NULL, ?3, ?3)",
            params![project, now - 1_000, now - id, id],
        )?;
    }
    insert_memory(
        &conn,
        100,
        project,
        Some("repo-owner-fact"),
        "decision",
        "Repo owner fact",
        "Opaque source body without signer terms.",
        now - 10_000,
    );
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
          confidence, supersedes_fact_id, status, invalidated_at_epoch,
          created_at_epoch, updated_at_epoch)
         VALUES (?1, 'HarborMint', 'verified_by', 'Toma Reed', ?2, NULL, ?3, 100,
                 NULL, '[]', 0.95, NULL, 'active', NULL, ?3, ?3)",
        params![project, now - 1_000, now - 10_000],
    )?;

    let memories = query_hybrid_context_memories(
        &conn,
        project,
        "Who signs HarborMint with Toma Reed?",
        None,
        &[],
        1,
    )?;

    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].title, "Repo owner fact");
    Ok(())
}

#[test]
fn hybrid_context_vector_recall_is_not_crowded_out_by_global_hits() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_context_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();
    let limits = ContextLimits {
        candidate_fetch_limit: 1,
        memory_index_limit: 10,
        core_item_limit: 3,
        ..ContextLimits::default()
    };
    let policy = ContextPolicy::from_limits(limits);

    insert_memory(
        &conn,
        1,
        project,
        Some("credential-store"),
        "architecture",
        "Credential store",
        "SQLCipher encrypts secrets at rest.",
        now - 10_000,
    );
    crate::retrieval::vector::upsert_memory_embedding_for_row(&conn, 1)?;
    for idx in 0..30 {
        let id = idx + 2;
        insert_global_memory(
            &conn,
            id,
            "global",
            Some(&format!("global-private-data-{idx}")),
            "bugfix",
            &format!("Global private data note {idx}"),
            "Protect private persisted data with a global-only diagnostic note.",
            now + idx,
        );
        crate::retrieval::vector::upsert_memory_embedding_for_row(&conn, id)?;
    }
    conn.execute(
        "INSERT INTO workstreams
         (id, project, title, status, next_action, created_at_epoch, updated_at_epoch)
         VALUES (1, ?1, 'Private persistence', 'active',
                 'How do we protect private persisted data?', ?2, ?2)",
        params![project, now],
    )?;

    let loaded = load_context_data_with_policy(&conn, project, None, &policy, true);

    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Credential store"));
    assert!(!loaded
        .memories
        .iter()
        .any(|memory| memory.title.starts_with("Global private data note")));
    Ok(())
}
