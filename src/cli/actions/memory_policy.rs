use anyhow::Result;
use serde::Serialize;

use crate::{db, memory::suppression};

use super::super::types::{MemoryAction, MemorySuppressionsAction};

pub(in crate::cli) fn run_memory_action(action: MemoryAction) -> Result<()> {
    match action {
        MemoryAction::Suppress {
            target,
            reason,
            actor,
            json,
        } => {
            let conn = db::open_db()?;
            let target = suppression::parse_target(&target)?;
            let suppression = suppression::create_suppression(
                &conn,
                &suppression::SuppressRequest {
                    target,
                    reason: reason.as_deref(),
                    actor: actor.as_deref(),
                },
            )?;
            if json {
                print_json(&SuppressionOutput {
                    status: "suppressed",
                    suppression,
                })?;
            } else {
                println!(
                    "Suppression {} active for {}.",
                    suppression.id,
                    target_label(&suppression)
                );
            }
        }
        MemoryAction::Unsuppress {
            target,
            reason,
            actor,
            json,
        } => {
            let conn = db::open_db()?;
            let suppressions = suppression::revoke_suppression_arg(
                &conn,
                &target,
                reason.as_deref(),
                actor.as_deref(),
            )?;
            if json {
                print_json(&UnsuppressionOutput {
                    status: "unsuppressed",
                    count: suppressions.len(),
                    suppressions,
                })?;
            } else {
                println!("Revoked {} suppression(s).", suppressions.len());
            }
        }
        MemoryAction::Feedback {
            target,
            value,
            source,
            context_injection_item_id,
            session_id,
            project,
            reason,
            json,
        } => {
            let conn = db::open_db()?;
            let target = suppression::parse_target(&target)?;
            let feedback = suppression::record_feedback(
                &conn,
                &suppression::FeedbackRequest {
                    target,
                    feedback: &value,
                    source: source.as_deref(),
                    context_injection_item_id,
                    session_id: session_id.as_deref(),
                    project: project.as_deref(),
                    reason: reason.as_deref(),
                },
            )?;
            if json {
                print_json(&FeedbackOutput {
                    status: "recorded",
                    feedback,
                })?;
            } else {
                println!("Feedback {} recorded.", feedback.id);
            }
        }
        MemoryAction::Suppressions {
            action:
                MemorySuppressionsAction::List {
                    include_inactive,
                    json,
                },
        } => {
            let conn = db::open_db()?;
            let suppressions = suppression::list_suppressions(&conn, include_inactive)?;
            if json {
                print_json(&SuppressionListOutput {
                    count: suppressions.len(),
                    suppressions,
                })?;
            } else if suppressions.is_empty() {
                println!("No memory suppressions found.");
            } else {
                for item in suppressions {
                    println!(
                        "{} [{}] {} {} ({})",
                        item.id,
                        item.status,
                        item.target_kind,
                        item.target_id
                            .map(|id| id.to_string())
                            .or(item.target_value)
                            .unwrap_or_default(),
                        item.reason
                    );
                }
            }
        }
        MemoryAction::Cleanup {
            cwd,
            cleanup_type,
            all_types,
            dry_run,
            plan_out,
            apply,
            plan,
            json,
        } => super::scope_cleanup::run_memory_cleanup(
            cwd.as_deref(),
            cleanup_type,
            all_types,
            dry_run,
            plan_out.as_deref(),
            apply,
            plan.as_deref(),
            json,
        )?,
    }
    Ok(())
}

fn target_label(record: &suppression::SuppressionRecord) -> String {
    record
        .target_id
        .map(|id| format!("{}:{id}", record.target_kind))
        .or_else(|| {
            record
                .target_value
                .as_ref()
                .map(|value| format!("{}:{value}", record.target_kind))
        })
        .unwrap_or_else(|| record.target_kind.clone())
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[derive(Serialize)]
struct SuppressionOutput {
    status: &'static str,
    suppression: suppression::SuppressionRecord,
}

#[derive(Serialize)]
struct UnsuppressionOutput {
    status: &'static str,
    count: usize,
    suppressions: Vec<suppression::SuppressionRecord>,
}

#[derive(Serialize)]
struct FeedbackOutput {
    status: &'static str,
    feedback: suppression::FeedbackRecord,
}

#[derive(Serialize)]
struct SuppressionListOutput {
    count: usize,
    suppressions: Vec<suppression::SuppressionRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::memory_types::MemorySuppressionsAction;
    use crate::memory;

    #[test]
    fn memory_policy_actions_write_suppressions_and_feedback() -> Result<()> {
        let _dir = crate::db::test_support::ScopedTestDataDir::new("memory-policy-cli-actions");
        let conn = db::open_db()?;
        memory::insert_memory(
            &conn,
            Some("s1"),
            "/repo",
            None,
            "Policy target",
            "Policy target body",
            "decision",
            None,
        )?;
        drop(conn);

        run_memory_action(MemoryAction::Suppress {
            target: "memory:1".to_string(),
            reason: Some("not relevant".to_string()),
            actor: Some("test".to_string()),
            json: true,
        })?;
        run_memory_action(MemoryAction::Feedback {
            target: "memory:1".to_string(),
            value: "not-relevant".to_string(),
            source: Some("test".to_string()),
            context_injection_item_id: None,
            session_id: Some("s1".to_string()),
            project: Some("/repo".to_string()),
            reason: Some("wrong task".to_string()),
            json: true,
        })?;
        run_memory_action(MemoryAction::Suppressions {
            action: MemorySuppressionsAction::List {
                include_inactive: false,
                json: true,
            },
        })?;

        let conn = db::open_db()?;
        let active: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_suppressions WHERE status = 'active'",
            [],
            |row| row.get(0),
        )?;
        let feedback: i64 =
            conn.query_row("SELECT COUNT(*) FROM memory_feedback", [], |row| row.get(0))?;
        assert_eq!(active, 1);
        assert_eq!(feedback, 1);
        drop(conn);

        run_memory_action(MemoryAction::Unsuppress {
            target: "memory:1".to_string(),
            reason: Some("needed again".to_string()),
            actor: Some("test".to_string()),
            json: true,
        })?;
        let conn = db::open_db()?;
        let active: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_suppressions WHERE status = 'active'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(active, 0);
        Ok(())
    }
}
