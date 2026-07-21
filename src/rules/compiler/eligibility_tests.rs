use anyhow::Result;

use super::tests::{compile, global_default, insert_pref, PrefSpec, PROJECT};
use super::{
    eligibility_decision, ClosedValue, EligibilityScope, RejectReason, RuleEligibilityDecision,
    RuleEligibilityInput, KNOWN_REVIEW, KNOWN_RISK, KNOWN_TRUST,
};
use crate::db::{self, test_support::ScopedTestDataDir};

// Behavior-based eligibility contract matrix (GH-813 / SP813-T2). Each test
// asserts only the compile / no-compile outcome for one eligibility dimension;
// none snapshot the SQL text. The closed contract is conjunctive: any single
// failing dimension, or any unknown enum value, must keep the source out.

#[test]
fn pure_eligibility_policy_is_conjunctive_and_table_driven() {
    let eligible = RuleEligibilityInput {
        memory_type: ClosedValue::Allowed,
        lifecycle: ClosedValue::Allowed,
        expires_at: None,
        scope: Some(EligibilityScope::Project),
        owner_scope: Some("repo"),
        owner_key: Some(PROJECT),
        target_project: None,
        legacy_project: PROJECT,
        current_project: PROJECT,
        trust: ClosedValue::Allowed,
        machine_checkable: 1,
        reinforcement_count: 3,
        min_reinforcement: 3,
        reinforcement_risk: ClosedValue::Allowed,
        candidate_risk: ClosedValue::Allowed,
        review: ClosedValue::Allowed,
        policy: ClosedValue::Allowed,
        now: 10,
    };
    assert_eq!(
        eligibility_decision(&eligible),
        RuleEligibilityDecision::Eligible
    );

    let cases = [
        (
            "type",
            RuleEligibilityInput {
                memory_type: ClosedValue::Denied,
                ..eligible
            },
            RejectReason::Type,
        ),
        (
            "lifecycle",
            RuleEligibilityInput {
                lifecycle: ClosedValue::Unknown,
                ..eligible
            },
            RejectReason::Lifecycle,
        ),
        (
            "expiry",
            RuleEligibilityInput {
                expires_at: Some(10),
                ..eligible
            },
            RejectReason::Expiry,
        ),
        (
            "scope",
            RuleEligibilityInput {
                scope: None,
                ..eligible
            },
            RejectReason::Scope,
        ),
        (
            "owner",
            RuleEligibilityInput {
                owner_scope: Some("user"),
                ..eligible
            },
            RejectReason::Owner,
        ),
        (
            "trust",
            RuleEligibilityInput {
                trust: ClosedValue::Unknown,
                ..eligible
            },
            RejectReason::Trust,
        ),
        (
            "machine",
            RuleEligibilityInput {
                machine_checkable: 0,
                ..eligible
            },
            RejectReason::MachineCheckable,
        ),
        (
            "threshold",
            RuleEligibilityInput {
                reinforcement_count: 2,
                ..eligible
            },
            RejectReason::Threshold,
        ),
        (
            "reinforcement_risk",
            RuleEligibilityInput {
                reinforcement_risk: ClosedValue::Denied,
                ..eligible
            },
            RejectReason::ReinforcementRisk,
        ),
        (
            "candidate_risk",
            RuleEligibilityInput {
                candidate_risk: ClosedValue::Denied,
                ..eligible
            },
            RejectReason::CandidateRisk,
        ),
        (
            "review",
            RuleEligibilityInput {
                review: ClosedValue::Unknown,
                ..eligible
            },
            RejectReason::Review,
        ),
        (
            "suppression",
            RuleEligibilityInput {
                policy: ClosedValue::Denied,
                ..eligible
            },
            RejectReason::Suppressed,
        ),
        (
            "malformed_policy",
            RuleEligibilityInput {
                policy: ClosedValue::Unknown,
                ..eligible
            },
            RejectReason::Policy,
        ),
    ];
    for (name, input, reason) in cases {
        assert_eq!(
            eligibility_decision(&input),
            RuleEligibilityDecision::Rejected(reason),
            "dimension={name}"
        );
    }
}

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
fn unknown_candidate_risk_with_low_reinforcement_risk_fails_closed() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-unknown-candidate-risk");
    let conn = db::open_db()?;
    // 'unknown' satisfies the schema CHECK constraint but is not classified
    // eligible by the contract, so it must fail closed rather than compile.
    insert_pref(
        &conn,
        &PrefSpec {
            candidate_risk: "unknown",
            ..Default::default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn unknown_reinforcement_risk_with_low_candidate_risk_fails_closed() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-unknown-reinforcement-risk");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            reinforcement_risk: "unknown",
            candidate_risk: "low",
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

#[test]
fn closed_enum_contract_lists_every_known_value() {
    assert_eq!(
        crate::memory::types::MemoryType::ALL.map(|value| value.as_str()),
        [
            "decision",
            "discovery",
            "bugfix",
            "architecture",
            "lesson",
            "preference",
            "procedure",
            "session_activity",
        ]
    );
    assert_eq!(
        KNOWN_TRUST,
        [
            "local_tool_output",
            "repo_file",
            "user_prompt",
            "external_content"
        ]
    );
    assert_eq!(KNOWN_RISK, ["low", "medium", "high", "unknown"]);
    assert_eq!(
        KNOWN_REVIEW,
        [
            "pending_review",
            "quarantined",
            "auto_promoted",
            "approved",
            "edited",
            "rejected",
            "discarded",
            "deferred",
        ]
    );
}

#[test]
fn every_allowed_trust_and_review_value_compiles() -> Result<()> {
    for (index, trust) in ["local_tool_output", "repo_file", "user_prompt"]
        .into_iter()
        .enumerate()
    {
        let _dir = ScopedTestDataDir::new(&format!("compile-allowed-trust-{index}"));
        let conn = db::open_db()?;
        insert_pref(
            &conn,
            &PrefSpec {
                source_trust_class: trust,
                ..Default::default()
            },
        )?;
        assert_eq!(compile(&conn)?.len(), 1, "trust={trust}");
    }
    for (index, review) in ["approved", "edited", "auto_promoted"]
        .into_iter()
        .enumerate()
    {
        let _dir = ScopedTestDataDir::new(&format!("compile-allowed-review-{index}"));
        let conn = db::open_db()?;
        insert_pref(
            &conn,
            &PrefSpec {
                review_status: review,
                ..Default::default()
            },
        )?;
        assert_eq!(compile(&conn)?.len(), 1, "review={review}");
    }
    Ok(())
}

#[test]
fn project_authority_uses_strict_target_owner_legacy_priority() -> Result<()> {
    let cases = [
        (Some("/tmp/wrong-target"), Some(PROJECT), false),
        (Some(PROJECT), Some("/tmp/wrong-owner"), true),
        (None, Some("/tmp/wrong-owner"), false),
        (None, None, true),
    ];
    for (index, (target, owner_key, expected)) in cases.into_iter().enumerate() {
        let _dir = ScopedTestDataDir::new(&format!("compile-owner-priority-{index}"));
        let conn = db::open_db()?;
        insert_pref(&conn, &PrefSpec::default())?;
        conn.execute(
            "UPDATE memories SET target_project = ?1, owner_key = ?2 WHERE id = 1",
            rusqlite::params![target, owner_key],
        )?;
        assert_eq!(!compile(&conn)?.is_empty(), expected, "case={index}");
    }
    Ok(())
}

#[test]
fn malformed_active_suppression_fails_closed() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-malformed-suppression");
    let conn = db::open_db()?;
    insert_pref(&conn, &PrefSpec::default())?;
    assert_eq!(compile(&conn)?.len(), 1);

    conn.pragma_update(None, "ignore_check_constraints", true)?;
    conn.execute(
        "INSERT INTO memory_suppressions
         (target_kind, target_id, target_value, reason, actor, status,
          created_at_epoch, updated_at_epoch)
         VALUES ('pattern', NULL, NULL, 'malformed', 'test', 'active', 1, 1)",
        [],
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn approved_candidate_does_not_mask_high_reinforcement_risk() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-approved-high-reinforcement-risk");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            review_status: "approved",
            reinforcement_risk: "high",
            candidate_risk: "low",
            ..Default::default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}

#[test]
fn low_risks_do_not_mask_unreviewed_candidate() -> Result<()> {
    let _dir = ScopedTestDataDir::new("compile-low-risk-unreviewed");
    let conn = db::open_db()?;
    insert_pref(
        &conn,
        &PrefSpec {
            review_status: "pending_review",
            reinforcement_risk: "low",
            candidate_risk: "low",
            ..Default::default()
        },
    )?;
    assert!(compile(&conn)?.is_empty());
    Ok(())
}
