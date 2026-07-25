use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::lesson::{save_lesson_with_outcome, LessonOutcomeUpdate, SaveLessonRequest};
use super::promote::slugify_for_topic;
use super::raw_archive::RawMessage;

const SOURCE: &str = "failure_trajectory_v1";
const MIN_LESSON_CONFIDENCE: f64 = 0.78;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FailureLessonFeedReport {
    pub inserted: usize,
    pub duplicates: usize,
    pub skipped: usize,
}

struct FailureLessonCandidate {
    title: String,
    lesson_text: String,
    evidence: String,
    evidence_ids: Vec<i64>,
    source_hash: String,
}

#[cfg(test)]
pub(crate) fn distill_session_failure_lessons(
    conn: &Connection,
    session_id: &str,
    project: &str,
    branch: Option<&str>,
) -> Result<FailureLessonFeedReport> {
    let messages = load_session_messages(conn, project, session_id)?;
    distill_messages(conn, session_id, project, branch, &messages)
}

pub(crate) fn distill_stop_failure_lessons(
    conn: &Connection,
    session_id: &str,
    project: &str,
    branch: Option<&str>,
    transcript_identity_ids: &[i64],
) -> Result<FailureLessonFeedReport> {
    let messages = load_stop_messages(conn, project, session_id, transcript_identity_ids)?;
    distill_messages(conn, session_id, project, branch, &messages)
}

fn distill_messages(
    conn: &Connection,
    session_id: &str,
    project: &str,
    branch: Option<&str>,
    messages: &[RawMessage],
) -> Result<FailureLessonFeedReport> {
    let Some(candidate) = detect_failure_lesson(messages) else {
        return Ok(FailureLessonFeedReport {
            skipped: usize::from(!messages.is_empty()),
            ..FailureLessonFeedReport::default()
        });
    };

    save_failure_lesson_candidate(conn, session_id, project, branch, &candidate)
}

fn load_stop_messages(
    conn: &Connection,
    project: &str,
    session_id: &str,
    transcript_identity_ids: &[i64],
) -> Result<Vec<RawMessage>> {
    if transcript_identity_ids.is_empty() {
        return load_session_messages(conn, project, session_id);
    }
    let placeholders = std::iter::repeat_n("?", transcript_identity_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT id, session_id, project, role, content, source, branch, cwd, created_at_epoch
         FROM raw_messages
         WHERE (project = ? AND session_id = ?)
            OR transcript_identity_id IN ({placeholders})
         ORDER BY id ASC"
    );
    let mut values: Vec<&dyn rusqlite::ToSql> =
        Vec::with_capacity(transcript_identity_ids.len() + 2);
    values.push(&project);
    values.push(&session_id);
    for identity_id in transcript_identity_ids {
        values.push(identity_id);
    }
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(values.as_slice(), raw_message_from_row)?;
    crate::db::query::collect_rows(rows)
}

fn load_session_messages(
    conn: &Connection,
    project: &str,
    session_id: &str,
) -> Result<Vec<RawMessage>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, project, role, content, source, branch, cwd, created_at_epoch
         FROM raw_messages
         WHERE project = ?1 AND session_id = ?2
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map(params![project, session_id], raw_message_from_row)?;
    crate::db::query::collect_rows(rows)
}

fn raw_message_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawMessage> {
    Ok(RawMessage {
        id: row.get(0)?,
        session_id: row.get(1)?,
        project: row.get(2)?,
        role: row.get(3)?,
        content: row.get(4)?,
        source: row.get(5)?,
        branch: row.get(6)?,
        cwd: row.get(7)?,
        created_at_epoch: row.get(8)?,
    })
}

fn detect_failure_lesson(messages: &[RawMessage]) -> Option<FailureLessonCandidate> {
    let failure = messages.iter().find_map(|message| {
        failure_evidence_sentence(&message.content).map(|text| (message, text))
    });
    let lesson = messages
        .iter()
        .find_map(|message| lesson_sentence(&message.content).map(|text| (message, text)));
    let (failure_message, failure_text) = failure?;
    let (lesson_message, lesson_text) = lesson?;
    let lesson_text = normalize_lesson_text(&lesson_text);
    let evidence = format!(
        "failure raw_message:{}: {}; lesson raw_message:{}: {}",
        failure_message.id,
        compact_sentence(&failure_text, 240),
        lesson_message.id,
        compact_sentence(&lesson_text, 360)
    );
    let evidence_ids = unique_ids([failure_message.id, lesson_message.id]);
    let source_hash = source_hash(&failure_text, &lesson_text);
    let title = title_for_lesson(&lesson_text);

    Some(FailureLessonCandidate {
        title,
        lesson_text,
        evidence,
        evidence_ids,
        source_hash,
    })
}

fn save_failure_lesson_candidate(
    conn: &Connection,
    session_id: &str,
    project: &str,
    branch: Option<&str>,
    candidate: &FailureLessonCandidate,
) -> Result<FailureLessonFeedReport> {
    conn.execute_batch("SAVEPOINT remem_failure_lesson_feed")
        .context("begin failure lesson feed savepoint")?;
    let result = save_failure_lesson_candidate_inner(conn, session_id, project, branch, candidate);
    match result {
        Ok(report) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_failure_lesson_feed")
                .context("release failure lesson feed savepoint")?;
            Ok(report)
        }
        Err(error) => {
            let rollback = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_failure_lesson_feed;
                 RELEASE SAVEPOINT remem_failure_lesson_feed;",
            );
            match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(error).context(format!(
                    "failure lesson feed rollback also failed: {rollback_error}"
                )),
            }
        }
    }
}

fn save_failure_lesson_candidate_inner(
    conn: &Connection,
    session_id: &str,
    project: &str,
    branch: Option<&str>,
    candidate: &FailureLessonCandidate,
) -> Result<FailureLessonFeedReport> {
    let now = chrono::Utc::now().timestamp();
    let evidence_json = serde_json::to_string(&candidate.evidence_ids)?;
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO memory_lesson_feed_events
         (project, session_id, source, source_hash, lesson_memory_id, outcome_kind,
          status, evidence_raw_message_ids, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, NULL, 'failure', 'pending', ?5, ?6, ?6)",
        params![
            project,
            session_id,
            SOURCE,
            candidate.source_hash,
            evidence_json,
            now
        ],
    )?;

    if inserted == 0 {
        return Ok(FailureLessonFeedReport {
            duplicates: 1,
            ..FailureLessonFeedReport::default()
        });
    }

    let event_id = conn.last_insert_rowid();
    let topic_key = format!(
        "failure-trajectory-{}",
        slugify_for_topic(&candidate.lesson_text, 72)
    );
    let memory_id = save_lesson_with_outcome(
        conn,
        &SaveLessonRequest {
            session_id: Some(session_id),
            project,
            topic_key: Some(&topic_key),
            title: &candidate.title,
            content: &candidate.lesson_text,
            confidence: MIN_LESSON_CONFIDENCE,
            source_evidence: Some(&candidate.evidence),
            files: None,
            branch,
            scope: "project",
            created_at_epoch: Some(now),
            stale_after_epoch: None,
        },
        LessonOutcomeUpdate::failure(),
    )?;

    conn.execute(
        "UPDATE memory_lesson_feed_events
         SET lesson_memory_id = ?1, status = 'saved', updated_at_epoch = ?2
         WHERE id = ?3",
        params![memory_id, now, event_id],
    )?;

    Ok(FailureLessonFeedReport {
        inserted: 1,
        ..FailureLessonFeedReport::default()
    })
}

fn failure_evidence_sentence(text: &str) -> Option<String> {
    split_sentences(text)
        .into_iter()
        .find(|sentence| has_failure_signal(sentence))
}

fn lesson_sentence(text: &str) -> Option<String> {
    split_sentences(text)
        .into_iter()
        .find(|sentence| has_explicit_lesson_signal(sentence))
}

fn split_sentences(text: &str) -> Vec<String> {
    text.lines()
        .flat_map(|line| line.split(['.', '!', '?']))
        .map(str::trim)
        .filter(|part| part.len() >= 24)
        .map(str::to_string)
        .collect()
}

fn has_failure_signal(sentence: &str) -> bool {
    let lower = sentence.to_ascii_lowercase();
    let has_command = contains_any(
        &lower,
        &[
            "cargo check",
            "cargo test",
            "cargo clippy",
            "cargo build",
            "npm test",
            "pnpm test",
            "pytest",
            "go test",
            "build",
            "compile",
            "test",
        ],
    );
    let has_failure = contains_any(
        &lower,
        &[
            "failed",
            "failure",
            "error",
            "panic",
            "exit code",
            "regression",
            "same compiler",
            "same error",
        ],
    );
    (has_command && has_failure)
        || lower.contains("error[")
        || lower.contains("compilation failed")
        || lower.contains("test failures")
}

fn has_explicit_lesson_signal(sentence: &str) -> bool {
    let lower = sentence.to_ascii_lowercase();
    contains_any(
        &lower,
        &[
            "stop and challenge",
            "challenge the hypothesis",
            "no fixes without root cause",
            "root cause first",
            "after three consecutive",
            "after 3 consecutive",
            "do not keep editing",
            "don't keep editing",
            "repeated failed fixes",
            "three failed fixes",
        ],
    )
}

fn normalize_lesson_text(sentence: &str) -> String {
    let trimmed = compact_sentence(sentence, 900);
    if trimmed.to_ascii_lowercase().starts_with("lesson:") {
        trimmed
    } else {
        format!("Lesson: {trimmed}")
    }
}

fn title_for_lesson(lesson_text: &str) -> String {
    let lower = lesson_text.to_ascii_lowercase();
    if lower.contains("challenge the hypothesis") || lower.contains("stop and challenge") {
        "Stop and challenge repeated failure loops".to_string()
    } else if lower.contains("root cause") {
        "Find root cause before more fixes".to_string()
    } else {
        "Learn from repeated failure signals".to_string()
    }
}

fn source_hash(failure_text: &str, lesson_text: &str) -> String {
    let normalized = format!(
        "{SOURCE}\n{}\n{}",
        normalize_for_hash(failure_text),
        normalize_for_hash(lesson_text)
    );
    crate::db::content_identity_hash(normalized.as_bytes())
}

fn normalize_for_hash(text: &str) -> String {
    text.to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn compact_sentence(text: &str, max_chars: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    compact.chars().take(max_chars).collect()
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn unique_ids(ids: [i64; 2]) -> Vec<i64> {
    let mut out = ids.to_vec();
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rusqlite::{params, Connection};

    use super::*;

    fn setup() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn insert_raw(conn: &Connection, session_id: &str, project: &str, role: &str, text: &str) {
        crate::memory::raw_archive::insert_raw_message(
            conn,
            session_id,
            project,
            role,
            text,
            crate::memory::raw_archive::SOURCE_TRANSCRIPT,
            Some("main"),
            Some(project),
        )
        .unwrap();
    }

    #[test]
    fn saves_failure_lesson_when_failure_and_explicit_lesson_share_session() -> Result<()> {
        let conn = setup()?;
        insert_raw(
            &conn,
            "s1",
            "/repo",
            crate::memory::raw_archive::ROLE_ASSISTANT,
            "cargo check failed with the same compiler error after the third attempted fix",
        );
        insert_raw(
            &conn,
            "s1",
            "/repo",
            crate::memory::raw_archive::ROLE_USER,
            "Lesson: after three consecutive failed fixes, stop and challenge the hypothesis before editing again",
        );

        let report = distill_session_failure_lessons(&conn, "s1", "/repo", Some("main"))?;

        assert_eq!(report.inserted, 1);
        let (outcome_kind, failure_count): (String, i64) = conn.query_row(
            "SELECT outcome_kind, failure_count
             FROM memory_lessons",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(outcome_kind, "failure");
        assert_eq!(failure_count, 1);
        Ok(())
    }

    #[test]
    fn duplicate_source_hash_does_not_reinforce_failure_count() -> Result<()> {
        let conn = setup()?;
        insert_raw(
            &conn,
            "s1",
            "/repo",
            crate::memory::raw_archive::ROLE_ASSISTANT,
            "cargo test failed with the same error after repeated edits",
        );
        insert_raw(
            &conn,
            "s1",
            "/repo",
            crate::memory::raw_archive::ROLE_USER,
            "Lesson: after three consecutive failed fixes, stop and challenge the hypothesis before editing again",
        );

        let first = distill_session_failure_lessons(&conn, "s1", "/repo", Some("main"))?;
        let second = distill_session_failure_lessons(&conn, "s1", "/repo", Some("main"))?;

        assert_eq!(first.inserted, 1);
        assert_eq!(second.duplicates, 1);
        let failure_count: i64 =
            conn.query_row("SELECT failure_count FROM memory_lessons", [], |row| {
                row.get(0)
            })?;
        let feed_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_lesson_feed_events",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(failure_count, 1);
        assert_eq!(feed_count, 1);
        Ok(())
    }

    #[test]
    fn failure_without_explicit_lesson_is_not_saved() -> Result<()> {
        let conn = setup()?;
        insert_raw(
            &conn,
            "s1",
            "/repo",
            crate::memory::raw_archive::ROLE_ASSISTANT,
            "cargo check failed with a compiler error in src/lib.rs",
        );

        let report = distill_session_failure_lessons(&conn, "s1", "/repo", Some("main"))?;

        assert_eq!(report.skipped, 1);
        assert_no_lessons(&conn)?;
        Ok(())
    }

    #[test]
    fn lesson_without_failure_evidence_is_not_saved() -> Result<()> {
        let conn = setup()?;
        insert_raw(
            &conn,
            "s1",
            "/repo",
            crate::memory::raw_archive::ROLE_USER,
            "Lesson: after three consecutive failed fixes, stop and challenge the hypothesis before editing again",
        );

        let report = distill_session_failure_lessons(&conn, "s1", "/repo", Some("main"))?;

        assert_eq!(report.skipped, 1);
        assert_no_lessons(&conn)?;
        Ok(())
    }

    fn assert_no_lessons(conn: &Connection) -> Result<()> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE memory_type = 'lesson'",
            [],
            |row| row.get(0),
        )?;
        let feed_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_lesson_feed_events",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 0);
        assert_eq!(feed_count, 0);
        Ok(())
    }

    #[test]
    fn source_hash_is_stable_under_whitespace() {
        assert_eq!(
            source_hash(
                "cargo check failed\nwith error",
                "Lesson: stop and challenge"
            ),
            source_hash(
                "cargo   check failed with error",
                "Lesson: stop and challenge"
            )
        );
    }

    #[test]
    fn feed_event_records_raw_message_ids() -> Result<()> {
        let conn = setup()?;
        insert_raw(
            &conn,
            "s1",
            "/repo",
            crate::memory::raw_archive::ROLE_ASSISTANT,
            "cargo check failed with the same compiler error after the third attempted fix",
        );
        insert_raw(
            &conn,
            "s1",
            "/repo",
            crate::memory::raw_archive::ROLE_USER,
            "Lesson: after three consecutive failed fixes, stop and challenge the hypothesis before editing again",
        );

        distill_session_failure_lessons(&conn, "s1", "/repo", Some("main"))?;

        let evidence: String = conn.query_row(
            "SELECT evidence_raw_message_ids FROM memory_lesson_feed_events",
            [],
            |row| row.get(0),
        )?;
        let ids: Vec<i64> = serde_json::from_str(&evidence)?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM raw_messages WHERE id IN (?1, ?2)",
            params![ids[0], ids[1]],
            |row| row.get(0),
        )?;
        assert_eq!(count, 2);
        Ok(())
    }
}
