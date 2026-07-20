use anyhow::Result;

use super::tests::{compile, global_default, insert_pref, PrefSpec};
use crate::db::{self, test_support::ScopedTestDataDir};

// Behavior-based eligibility contract matrix (GH-813 / SP813-T2). Each test
// asserts only the compile / no-compile outcome for one eligibility dimension;
// none snapshot the SQL text. The closed contract is conjunctive: any single
// failing dimension, or any unknown enum value, must keep the source out.

#[test]
fn eligible_global_user_default_preference_compiles() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-global-eligible");
    let conn = db::open_db()?;
    insert_pref(&conn, &global_default())?;

    assert_eq!(compile(&conn)?, vec!["pref-1-1".to_string()]);
    Ok(())
}

#[test]
fn non_preference_memory_type_is_not_compiled() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-memory-type");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            memory_type: "lesson",
            ..Default::default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

// GH-813 regression: a global row is eligible only for the exact
// owner_scope='user' / owner_key='user:default' / no-target tuple. Each of the
// three deviations below must fail closed.

#[test]
fn global_preference_with_wrong_owner_scope_is_not_compiled() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-global-wrong-owner-scope");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            owner_scope: Some("repo"),
            ..global_default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn global_preference_with_wrong_owner_key_is_not_compiled() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-global-wrong-owner-key");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            owner_key: Some("user:other"),
            ..global_default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn global_preference_with_project_target_is_not_compiled() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-global-project-target");
    let conn = db::open_db()?;
    insert_pref(&conn, &global_default())?;
    conn.execute(
        "UPDATE memories SET target_project = ?1 WHERE id = 1",
        ["/tmp/other-project"],
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

// Unknown / closed-enum values fail closed: a value the contract has not
// explicitly classified is ineligible rather than accidentally eligible.

#[test]
fn global_preference_with_unknown_owner_scope_fails_closed() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-global-unknown-owner-scope");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            owner_scope: Some("team"),
            ..global_default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn unknown_scope_value_fails_closed() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-unknown-scope");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            scope: "workspace",
            ..Default::default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn unknown_risk_class_value_fails_closed() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-unknown-risk");
    let conn = db::open_db()?;
    // 'unknown' satisfies the schema CHECK constraint but is not classified
    // eligible by the contract, so it must fail closed rather than compile.
    insert_pref(
        &conn,
        &PrefSpec {
            risk_class: "unknown",
            ..Default::default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn unknown_review_status_value_fails_closed() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-unknown-review-status");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            review_status: "unknown",
            ..Default::default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn unknown_source_trust_value_fails_closed() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-unknown-source-trust");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            source_trust_class: "web_scrape",
            ..Default::default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

// Candidate risk and reinforcement risk are independent inputs: each alone
// must gate eligibility even when the other stays low.

#[test]
fn high_reinforcement_risk_with_low_candidate_risk_is_not_compiled() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-reinforcement-risk-independent");
    let conn = db::open_db()?;
    insert_pref(&conn, &PrefSpec::default())?;
    assert_eq!(compile(&conn)?.len(), 1);

    conn.execute(
        "UPDATE memory_preference_reinforcements SET risk_class = 'high' WHERE memory_id = 1",
        [],
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn high_candidate_risk_with_low_reinforcement_risk_is_not_compiled() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-candidate-risk-independent");
    let conn = db::open_db()?;
    insert_pref(&conn, &PrefSpec::default())?;
    assert_eq!(compile(&conn)?.len(), 1);

    conn.execute(
        "UPDATE memory_candidates SET risk_class = 'high' WHERE id = 1",
        [],
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn risk_reclassification_across_states_removes_rule() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-risk-cross-state");
    let conn = db::open_db()?;
    insert_pref(&conn, &PrefSpec::default())?;
    assert_eq!(compile(&conn)?.len(), 1);

    conn.execute(
        "UPDATE memory_candidates SET risk_class = 'high' WHERE id = 1",
        [],
    )?;
    assert!(compile(&conn)?.is_empty());

    conn.execute(
        "UPDATE memory_candidates SET risk_class = 'low' WHERE id = 1",
        [],
    )?;
    assert_eq!(compile(&conn)?.len(), 1);
    Ok(())
}
