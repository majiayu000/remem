use super::{CandidateSourceBatch, ParsedUserContextCandidate};
use crate::user_context::non_retention::prevention;

pub(super) fn is_supported(
    candidate: &ParsedUserContextCandidate,
    batch: &CandidateSourceBatch,
) -> bool {
    if candidate.source_kind != "explicit_user_statement"
        || !matches!(
            candidate.claim_type,
            super::super::claims::UserContextClaimType::Preference
                | super::super::claims::UserContextClaimType::Constraint
        )
    {
        return false;
    }
    let Some(key) = prevention::constraint_key(&candidate.claim_text) else {
        return false;
    };
    let events = batch.events_for_candidate(candidate);
    !events.is_empty()
        && events.iter().all(|event| {
            batch.event_is_user_authored(event.id)
                && event.tool_name.is_none()
                && matches!(event.event_type.as_str(), "message" | "user_prompt_submit")
                && prevention::constraint_key(&event.content) == Some(key)
        })
}

pub(super) fn block_reason(
    candidate: &ParsedUserContextCandidate,
    batch: &CandidateSourceBatch,
) -> Option<&'static str> {
    (prevention::constraint_key(&candidate.claim_text).is_some() && !is_supported(candidate, batch))
        .then_some("no_supporting_user_source_event")
}

pub(super) const REVIEW_REASON: &str = "preventive_security_constraint_requires_review";

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rusqlite::Connection;

    use crate::db::{self, CaptureEventInput, ExtractionTaskKind};
    use crate::user_context::extraction::process_with_generator;

    async fn extract(
        claim: &str,
        sources: &[(Option<&str>, &str)],
        event_type: &str,
        tool_name: Option<&str>,
    ) -> Result<Connection> {
        let mut conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let mut event_ids = Vec::new();
        for (role, content) in sources {
            let outcome = db::record_captured_event(
                &conn,
                &CaptureEventInput {
                    host: "codex-cli",
                    session_id: "preventive-constraint",
                    project: "/tmp/remem-preventive",
                    cwd: None,
                    event_type,
                    role: *role,
                    tool_name,
                    content,
                    task_kind: Some(ExtractionTaskKind::UserContextCandidate),
                },
            )?;
            event_ids.push(outcome.event_row_id);
        }
        let task = db::claim_next_extraction_task(&mut conn, "preventive-test", 60)?
            .expect("candidate task");
        let response = serde_json::json!({"candidates": [{
            "claim_type": "constraint",
            "claim_key": "constraint:security",
            "claim_text": claim,
            "confidence": 0.99,
            "sensitivity": "normal",
            "risk_class": "low",
            "source_kind": "explicit_user_statement",
            "source_event_ids": event_ids,
        }]})
        .to_string();
        process_with_generator(&mut conn, &task, |_| async move { Ok(response) }).await?;
        Ok(conn)
    }

    fn candidate_count(conn: &Connection) -> Result<i64> {
        Ok(
            conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
                row.get(0)
            })?,
        )
    }

    #[tokio::test]
    async fn explicit_preventive_constraints_reach_review_with_exact_user_evidence() -> Result<()> {
        for (claim, source) in [
            (
                "User must not bypass authentication.",
                "I must not bypass authentication.",
            ),
            (
                "User never wants to bypass authentication.",
                "I never want to bypass authentication.",
            ),
            (
                "Never bypass authentication.",
                "Never bypass authentication.",
            ),
            (
                "User requires agents never to bypass authentication.",
                "I require agents never to bypass authentication.",
            ),
            (
                "User requires malware prevention.",
                "I require malware prevention.",
            ),
            ("用户要求不要绕过认证。", "我要求不要绕过认证。"),
            ("用户禁止窃取凭证。", "我禁止窃取凭证。"),
            ("用户要求防范网络钓鱼。", "我要求防范网络钓鱼。"),
        ] {
            let conn = extract(claim, &[(Some("user"), source)], "message", None).await?;
            assert_eq!(candidate_count(&conn)?, 1, "{claim}");
            let (status, reason, preview): (String, String, String) = conn.query_row(
                "SELECT review_status, auto_promote_block_reason, source_preview FROM user_context_candidates",
                [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            assert_eq!(status, "pending_review");
            assert_eq!(reason, super::REVIEW_REASON);
            assert_eq!(preview, source);
            let active: i64 =
                conn.query_row("SELECT COUNT(*) FROM user_context_claims", [], |row| {
                    row.get(0)
                })?;
            assert_eq!(active, 0);
        }
        Ok(())
    }

    #[tokio::test]
    async fn repeated_preventive_sources_keep_a_complete_preview_and_validate_every_citation(
    ) -> Result<()> {
        for (claim, source, unsafe_source) in [
            (
                "Never bypass authentication.",
                "Never bypass authentication.",
                "I want to bypass authentication.",
            ),
            ("不要绕过认证。", "不要绕过认证。", "我想绕过认证。"),
        ] {
            let mut sources = vec![(Some("user"), source); 80];
            assert!(
                sources
                    .iter()
                    .map(|(_, text)| text.chars().count())
                    .sum::<usize>()
                    > 500
            );
            let conn = extract(claim, &sources, "message", None).await?;
            assert_eq!(candidate_count(&conn)?, 1);
            let (status, preview, refs): (String, String, String) = conn.query_row(
                "SELECT review_status, source_preview, source_refs_json FROM user_context_candidates",
                [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            assert_eq!(status, "pending_review");
            assert_eq!(preview, source);
            assert_eq!(
                serde_json::from_str::<Vec<serde_json::Value>>(&refs)?.len(),
                sources.len()
            );

            sources.push((Some("user"), unsafe_source));
            let conn = extract(claim, &sources, "message", None).await?;
            assert_eq!(
                candidate_count(&conn)?,
                0,
                "later citations must still be checked"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn long_preventive_source_whitespace_keeps_review_and_other_candidates() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let preventive_source = format!("Never{}bypass authentication.", " \t\n".repeat(200));
        let ordinary_source = "I prefer   concise\tcode reviews.";
        assert!(preventive_source.len() > 500);
        let sources = [preventive_source.as_str(), ordinary_source];
        let mut event_ids = Vec::new();
        for content in sources {
            let outcome = db::record_captured_event(
                &conn,
                &CaptureEventInput {
                    host: "codex-cli",
                    session_id: "preventive-whitespace",
                    project: "/tmp/remem-preventive",
                    cwd: None,
                    event_type: "message",
                    role: Some("user"),
                    tool_name: None,
                    content,
                    task_kind: Some(ExtractionTaskKind::UserContextCandidate),
                },
            )?;
            event_ids.push(outcome.event_row_id);
        }
        let task = db::claim_next_extraction_task(&mut conn, "preventive-test", 60)?
            .expect("candidate task");
        let candidates = [
            (
                "constraint",
                "constraint:security",
                "Never bypass authentication.",
            ),
            (
                "preference",
                "preference:review-style",
                "User prefers concise code reviews.",
            ),
        ]
        .into_iter()
        .zip(&event_ids)
        .map(|((claim_type, claim_key, claim_text), event_id)| {
            serde_json::json!({
                "claim_type": claim_type,
                "claim_key": claim_key,
                "claim_text": claim_text,
                "confidence": 0.99,
                "sensitivity": "normal",
                "risk_class": "low",
                "source_kind": "explicit_user_statement",
                "source_event_ids": [event_id],
            })
        })
        .collect::<Vec<_>>();
        let response = serde_json::json!({"candidates": candidates}).to_string();
        let result =
            process_with_generator(&mut conn, &task, |_| async move { Ok(response) }).await?;
        assert_eq!(
            result,
            super::super::UserContextCandidateExtractResult::Written {
                candidates: 2,
                promoted: 1,
                pending_review: 1,
                to_event_id: event_ids[1],
            }
        );
        for (index, key, expected_status, expected_reason, expected_preview) in [
            (
                0,
                "constraint:security",
                "pending_review",
                Some(super::REVIEW_REASON),
                "Never bypass authentication.",
            ),
            (
                1,
                "preference:review-style",
                "auto_promoted",
                None,
                ordinary_source,
            ),
        ] {
            let (status, reason, preview, refs): (String, Option<String>, String, String) = conn
                .query_row(
                "SELECT review_status, auto_promote_block_reason, source_preview, source_refs_json
                     FROM user_context_candidates WHERE claim_key = ?1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
            assert_eq!(status, expected_status);
            assert_eq!(reason.as_deref(), expected_reason);
            assert_eq!(preview, expected_preview);
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&refs)?,
                serde_json::json!([{"kind": "captured_event", "id": event_ids[index]}])
            );
            let raw: String = conn.query_row(
                "SELECT content_text FROM captured_events WHERE id = ?1",
                [event_ids[index]],
                |row| row.get(0),
            )?;
            assert_eq!(raw, sources[index], "raw source must remain unchanged");
        }
        Ok(())
    }

    #[tokio::test]
    async fn harmful_positive_quoted_double_negative_and_secret_sources_create_no_candidate(
    ) -> Result<()> {
        for (claim, source) in [
            (
                "User wants to bypass authentication.",
                "I want to bypass authentication.",
            ),
            (
                "Never bypass authentication.",
                "I do not want to never bypass authentication.",
            ),
            (
                "Never bypass authentication.",
                "\"Never bypass authentication.\"",
            ),
            (
                "Never bypass authentication.",
                "I never bypass authentication unless needed.",
            ),
            (
                "Never bypass authentication.",
                "I never bypass authentication. I want to bypass authentication.",
            ),
            (
                "Never bypass authentication.",
                "I never bypass authentication. API key is sk-testsecret123456.",
            ),
            (
                "User works on malware analysis.",
                "I work on malware analysis.",
            ),
            ("用户想绕过认证。", "我想绕过认证。"),
            ("不要绕过认证。", "我不是不要绕过认证。"),
            ("不要绕过认证。", "“不要绕过认证。”"),
            ("不要绕过认证。", "我不要绕过认证，除非需要。"),
            ("不要绕过认证。", "我不要绕过认证。我想绕过认证。"),
            (
                "不要绕过认证。",
                "不要绕过认证。 API key is sk-testsecret123456.",
            ),
            ("用户从事恶意软件分析。", "我从事恶意软件分析。"),
        ] {
            let conn = extract(claim, &[(Some("user"), source)], "message", None).await?;
            assert_eq!(candidate_count(&conn)?, 0, "{source}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn preventive_exception_rejects_external_tool_and_mixed_provenance() -> Result<()> {
        for statement in ["Never bypass authentication.", "不要绕过认证。"] {
            for role in [Some("assistant"), Some("tool"), None] {
                let conn = extract(statement, &[(role, statement)], "message", None).await?;
                assert_eq!(candidate_count(&conn)?, 0);
            }
            let conn = extract(
                statement,
                &[(Some("user"), statement)],
                "file_read",
                Some("Read"),
            )
            .await?;
            assert_eq!(candidate_count(&conn)?, 0, "a file is not a user statement");
            let conn = extract(
                statement,
                &[(Some("user"), statement), (Some("assistant"), statement)],
                "message",
                None,
            )
            .await?;
            assert_eq!(
                candidate_count(&conn)?,
                0,
                "mixed citations must not inherit the exception"
            );
        }
        for (claim, source) in [
            (
                "Never bypass authentication.",
                "The README says: never bypass authentication.",
            ),
            ("不要绕过认证。", "网页说：不要绕过认证。"),
        ] {
            let conn = extract(claim, &[(Some("user"), source)], "message", None).await?;
            assert_eq!(candidate_count(&conn)?, 0);
        }
        Ok(())
    }
}
