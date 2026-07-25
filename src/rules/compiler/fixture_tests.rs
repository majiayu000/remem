use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::json;

use super::compile_project_rules;
use crate::db::{self, test_support::ScopedTestDataDir};
use crate::rules::{
    artifact_path_for_project, evaluate_pre_tool_use, write_artifact_atomic, CompiledRulesArtifact,
};
use crate::runtime_config::RuleCompilationConfig;

const FIXTURES: &str =
    include_str!("../../../tests/fixtures/rule-enforcement-repeated-corrections.json");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureSuite {
    schema_version: u32,
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
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
fn repeated_corrections_compile_and_warn_end_to_end() -> Result<()> {
    let test_dir = ScopedTestDataDir::new("repeated-correction-fixtures");
    let project_dir = test_dir.path.join("project");
    std::fs::create_dir_all(&project_dir)?;
    let project = db::project_from_cwd(&project_dir.to_string_lossy());
    let suite: FixtureSuite = serde_json::from_str(FIXTURES)?;
    ensure!(suite.schema_version == 1, "unsupported fixture schema");
    ensure!(suite.scenarios.len() == 3, "expected three fixture classes");
    let conn = db::open_db()?;
    for scenario in &suite.scenarios {
        insert_reinforced_preference(&conn, &project, scenario)?;
    }

    let artifact = compile_project_rules(
        &conn,
        &project,
        RuleCompilationConfig {
            enabled: true,
            min_reinforcement: 3,
        },
    )?;
    ensure!(
        artifact.rules.len() == suite.scenarios.len(),
        "each repeated correction must compile exactly one rule"
    );
    for scenario in &suite.scenarios {
        ensure!(
            artifact.rules.iter().any(|rule| {
                rule.source_memory_id == scenario.source_memory_id
                    && rule.reinforcement_count == scenario.reinforcement_count
            }),
            "{} did not preserve compiler provenance",
            scenario.id
        );
    }

    let data_dir = db::absolute_data_dir()?;
    let artifact_path = artifact_path_for_project(&data_dir, &project);
    write_artifact_atomic(&artifact_path, &CompiledRulesArtifact::new(1, Vec::new()))?;
    for scenario in &suite.scenarios {
        let outcome = evaluate_pre_tool_use(
            &fixture_hook_input(&project, &scenario.id, &scenario.violating_command),
            Some("claude-code"),
            &data_dir,
            true,
        )?;
        ensure!(
            outcome.output.is_none() && outcome.diagnostics.is_empty(),
            "{} warned without a compiled rule",
            scenario.id
        );
    }

    write_artifact_atomic(&artifact_path, &artifact)?;
    for scenario in &suite.scenarios {
        let violation = evaluate_pre_tool_use(
            &fixture_hook_input(&project, &scenario.id, &scenario.violating_command),
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

        let compliant = evaluate_pre_tool_use(
            &fixture_hook_input(&project, &scenario.id, &scenario.compliant_command),
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

fn insert_reinforced_preference(
    conn: &Connection,
    project: &str,
    scenario: &Scenario,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_candidates
         (id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch,
          source_trust_class)
         VALUES (?1, 'project', 'preference', ?2, ?3, '[1]',
                 0.95, 'low', 'auto_promoted', 1, 1, 'local_tool_output')",
        params![
            scenario.source_memory_id,
            format!("fixture-{}", scenario.id),
            scenario.correction,
        ],
    )?;
    conn.execute(
        "INSERT INTO memories
         (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch,
          status, scope, owner_scope, owner_key, source_candidate_id, source_trust_class)
         VALUES (?1, ?2, 'fixture preference', ?3, 'preference', 1, 1,
                 'active', 'project', 'repo', ?2, ?1, 'local_tool_output')",
        params![scenario.source_memory_id, project, scenario.correction],
    )?;
    conn.execute(
        "INSERT INTO memory_preference_reinforcements
         (memory_id, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, created_at_epoch, updated_at_epoch,
          machine_checkable, risk_class)
         VALUES (?1, ?2, NULL, 1, 1, 1, 1, 'low')",
        params![scenario.source_memory_id, scenario.reinforcement_count],
    )?;
    Ok(())
}

fn fixture_hook_input(project: &str, session_id: &str, command: &str) -> String {
    json!({
        "session_id": session_id,
        "cwd": project,
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": command}
    })
    .to_string()
}
