use super::*;

fn setup_fact_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

fn input<'a>(project: &'a str, object: &'a str, valid_from_epoch: i64) -> TemporalFactInput<'a> {
    TemporalFactInput {
        project,
        subject: "deploy-target",
        predicate: FactPredicate::AffectsProject,
        object,
        valid_from_epoch: Some(valid_from_epoch),
        valid_to_epoch: None,
        learned_at_epoch: Some(valid_from_epoch),
        source_memory_id: None,
        source_observation_id: None,
        source_event_ids: &[],
        confidence: 0.9,
        supersedes_fact_id: None,
    }
}

#[test]
fn supersession_preserves_historical_fact_but_hides_it_from_current() -> Result<()> {
    let mut conn = setup_fact_conn()?;
    let project = "test-temporal";
    let old_id = insert_temporal_fact(&mut conn, &input(project, "staging", 100))?;
    let mut replacement = input(project, "production", 200);
    replacement.supersedes_fact_id = Some(old_id);
    let new_id = insert_temporal_fact(&mut conn, &replacement)?;

    let current = list_current_facts(
        &conn,
        project,
        Some("deploy-target"),
        Some(FactPredicate::AffectsProject),
    )?;
    assert_eq!(
        current.iter().map(|fact| fact.id).collect::<Vec<_>>(),
        vec![new_id]
    );
    assert_eq!(current[0].object, "production");

    let old: (String, Option<i64>, Option<i64>) = conn.query_row(
        "SELECT status, valid_to_epoch, invalidated_at_epoch
         FROM memory_facts WHERE id = ?1",
        [old_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(old.0, "stale");
    assert_eq!(old.1, Some(200));
    assert!(old.2.is_some());

    let before = list_facts_as_of(
        &conn,
        project,
        150,
        Some("deploy-target"),
        Some(FactPredicate::AffectsProject),
    )?;
    assert_eq!(before[0].object, "staging");

    let after = list_facts_as_of(
        &conn,
        project,
        250,
        Some("deploy-target"),
        Some(FactPredicate::AffectsProject),
    )?;
    assert_eq!(after[0].object, "production");
    Ok(())
}

#[test]
fn supersession_records_transaction_time_invalidation() -> Result<()> {
    let conn = setup_fact_conn()?;
    let project = "test-temporal-invalidated";
    let old_id = insert_temporal_fact_in_current_tx(&conn, &input(project, "staging", 100), 150)?;
    let mut replacement = input(project, "production", 200);
    replacement.supersedes_fact_id = Some(old_id);
    let new_id = insert_temporal_fact_in_current_tx(&conn, &replacement, 250)?;

    let old: (String, Option<i64>, Option<i64>, i64) = conn.query_row(
        "SELECT status, valid_to_epoch, invalidated_at_epoch, updated_at_epoch
         FROM memory_facts WHERE id = ?1",
        [old_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(old, ("stale".to_string(), Some(200), Some(250), 250));

    let replacement_invalidated_at: Option<i64> = conn.query_row(
        "SELECT invalidated_at_epoch FROM memory_facts WHERE id = ?1",
        [new_id],
        |row| row.get(0),
    )?;
    assert_eq!(replacement_invalidated_at, None);
    Ok(())
}

#[test]
fn provenance_links_to_source_memory_and_events() -> Result<()> {
    let mut conn = setup_fact_conn()?;
    let memory_id = crate::memory::insert_memory(
        &conn,
        Some("session-a"),
        "test-provenance",
        Some("verification-command"),
        "Verification command",
        "The deploy fix was verified with cargo test.",
        "bugfix",
        None,
    )?;
    let observation_id = crate::db::insert_observation(
        &conn,
        "session-a",
        "test-provenance",
        "bugfix",
        Some("Deploy fix"),
        None,
        Some("The deploy fix was verified with cargo test."),
        None,
        None,
        None,
        None,
        Some(1),
        0,
    )?;
    let input = TemporalFactInput {
        project: "test-provenance",
        subject: "deploy-fix",
        predicate: FactPredicate::VerifiedBy,
        object: "cargo test",
        valid_from_epoch: Some(300),
        valid_to_epoch: None,
        learned_at_epoch: Some(320),
        source_memory_id: Some(memory_id),
        source_observation_id: Some(observation_id),
        source_event_ids: &[11, 12],
        confidence: 0.95,
        supersedes_fact_id: None,
    };
    let id = insert_temporal_fact(&mut conn, &input)?;

    let facts = list_current_facts(
        &conn,
        "test-provenance",
        Some("deploy-fix"),
        Some(FactPredicate::VerifiedBy),
    )?;
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].id, id);
    assert_eq!(facts[0].source_memory_id, Some(memory_id));
    assert_eq!(facts[0].source_observation_id, Some(observation_id));
    assert_eq!(facts[0].source_event_ids, vec![11, 12]);
    assert_eq!(facts[0].confidence, 0.95);
    Ok(())
}

#[test]
fn rejects_invalid_validity_window() -> Result<()> {
    let mut conn = setup_fact_conn()?;
    let mut input = input("test-invalid", "production", 500);
    input.valid_to_epoch = Some(400);
    let err = insert_temporal_fact(&mut conn, &input).expect_err("invalid time must fail");
    assert!(err.to_string().contains("valid_to_epoch"));
    Ok(())
}

#[test]
fn rejects_supersession_cutoff_before_old_valid_from() -> Result<()> {
    let mut conn = setup_fact_conn()?;
    let project = "test-temporal-cutoff";
    let old_id = insert_temporal_fact(&mut conn, &input(project, "staging", 500))?;
    let mut replacement = input(project, "production", 400);
    replacement.supersedes_fact_id = Some(old_id);

    let err = insert_temporal_fact(&mut conn, &replacement)
        .expect_err("backdated supersession cutoff should fail before mutation");
    assert!(err.to_string().contains("before existing valid_from_epoch"));

    let (status, valid_to): (String, Option<i64>) = conn.query_row(
        "SELECT status, valid_to_epoch FROM memory_facts WHERE id = ?1",
        [old_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_facts WHERE project = ?1",
        [project],
        |row| row.get(0),
    )?;
    assert_eq!(status, "active");
    assert_eq!(valid_to, None);
    assert_eq!(count, 1);
    Ok(())
}
