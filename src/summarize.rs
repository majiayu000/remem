use anyhow::Result;
use serde::Deserialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::db;
use crate::memory_format;
const SUMMARY_PROMPT: &str = include_str!("../prompts/summary.txt");

/// 同项目 summarize 冷却期（秒）
const SUMMARIZE_COOLDOWN_SECS: i64 = 300;

/// Lock timeout for summary processing (seconds).
const SUMMARIZE_LOCK_TIMEOUT_SECS: i64 = 180;

/// Stop hook stdin read timeout. Some runners may keep stdin open on edge cases.
const SUMMARIZE_STDIN_TIMEOUT_MS: u64 = 3000;

fn hash_message(msg: &str) -> String {
    let mut hasher = DefaultHasher::new();
    msg.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[derive(Debug, Deserialize)]
struct SummarizeInput {
    session_id: Option<String>,
    cwd: Option<String>,
    transcript_path: Option<String>,
    last_assistant_message: Option<String>,
}

use crate::db::project_from_cwd;

fn extract_last_assistant_message(transcript_path: &str) -> Option<String> {
    let content = std::fs::read_to_string(transcript_path).ok()?;
    let mut last_assistant = None;

    for line in content.lines().rev() {
        let val: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // skip malformed lines instead of aborting
        };
        if val["type"].as_str() == Some("assistant") {
            let Some(content_arr) = val["message"]["content"].as_array() else {
                continue;
            };
            let text_parts: Vec<&str> = content_arr
                .iter()
                .filter_map(|c| {
                    if c["type"].as_str() == Some("text") {
                        c["text"].as_str()
                    } else {
                        None
                    }
                })
                .collect();
            if !text_parts.is_empty() {
                last_assistant = Some(text_parts.join("\n"));
                break;
            }
        }
    }
    last_assistant
}

pub struct ParsedSummary {
    pub request: Option<String>,
    pub completed: Option<String>,
    pub decisions: Option<String>,
    pub learned: Option<String>,
    pub next_steps: Option<String>,
    pub preferences: Option<String>,
}

pub fn parse_summary(text: &str) -> Option<ParsedSummary> {
    if text.contains("<skip_summary") {
        return None;
    }

    let start = text.find("<summary>")? + "<summary>".len();
    let end = start + text[start..].find("</summary>")?;
    let content = &text[start..end];

    Some(ParsedSummary {
        request: memory_format::extract_field(content, "request"),
        completed: memory_format::extract_field(content, "completed"),
        decisions: memory_format::extract_field(content, "decisions"),
        learned: memory_format::extract_field(content, "learned"),
        next_steps: memory_format::extract_field(content, "next_steps"),
        preferences: memory_format::extract_field(content, "preferences"),
    })
}

fn read_stdin_with_timeout(timeout_ms: u64) -> Result<Option<String>> {
    use std::sync::mpsc;
    use std::time::Duration;

    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let input = std::io::read_to_string(std::io::stdin());
        let _ = tx.send(input);
    });

    match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(Ok(input)) => {
            if input.trim().is_empty() {
                Ok(None)
            } else {
                Ok(Some(input))
            }
        }
        Ok(Err(e)) => Err(e.into()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            crate::log::warn(
                "summarize",
                &format!("stdin read timed out after {}ms, skipping", timeout_ms),
            );
            Ok(None)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            crate::log::warn("summarize", "stdin reader disconnected, skipping");
            Ok(None)
        }
    }
}

/// Stop hook entry: enqueue summary job and return immediately.
pub async fn summarize() -> Result<()> {
    let Some(input) = read_stdin_with_timeout(SUMMARIZE_STDIN_TIMEOUT_MS)? else {
        return Ok(());
    };

    // Quick validation
    let hook: SummarizeInput = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(e) => {
            crate::log::warn("summarize", &format!("invalid hook payload, skipping: {}", e));
            return Ok(());
        }
    };
    let Some(session_id) = &hook.session_id else {
        return Ok(());
    };
    let cwd = hook.cwd.as_deref().unwrap_or(".");
    let project = db::project_from_cwd(cwd);
    let conn = db::open_db()?;

    // Enqueue observation flush job (high priority)
    let obs_payload = serde_json::json!({
        "session_id": session_id,
        "project": project,
    });
    db::enqueue_job(
        &conn,
        db::JobType::Observation,
        &project,
        Some(session_id),
        &obs_payload.to_string(),
        50,
    )?;

    db::enqueue_job(
        &conn,
        db::JobType::Summary,
        &project,
        Some(session_id),
        &input,
        100,
    )?;
    // Low-priority maintenance job: deduped per project by enqueue_job.
    db::enqueue_job(
        &conn,
        db::JobType::Compress,
        &project,
        None,
        "{}",
        200,
    )?;
    crate::log::info(
        "summarize",
        &format!("QUEUED observation+summary session={} project={}", session_id, project),
    );
    // Kick one worker cycle in background so hooks stay non-blocking.
    let exe = std::env::current_exe()?;
    let stderr_file = crate::log::open_log_append();
    let stderr_cfg = match stderr_file {
        Some(f) => std::process::Stdio::from(f),
        None => std::process::Stdio::null(),
    };
    let _child = std::process::Command::new(&exe)
        .arg("worker")
        .arg("--once")
        .env("REMEM_STDERR_TO_LOG", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr_cfg)
        .spawn()?;
    Ok(())
}

/// Process one queued summary job payload (hook JSON).
/// This path only does summary generation and DB write; it does not run maintenance tasks.
pub async fn process_summary_job_input(input: &str) -> Result<()> {
    let hook: SummarizeInput = serde_json::from_str(input)?;
    let Some(session_id) = hook.session_id else {
        return Ok(());
    };
    let cwd = hook.cwd.as_deref().unwrap_or(".");
    let project = project_from_cwd(cwd);

    let assistant_msg = hook
        .last_assistant_message
        .or_else(|| {
            hook.transcript_path
                .as_deref()
                .and_then(extract_last_assistant_message)
        })
        .unwrap_or_default();

    if assistant_msg.is_empty() {
        return Ok(());
    }
    if assistant_msg.contains("<skip_summary") || assistant_msg.len() < 50 {
        return Ok(());
    }

    let msg = if assistant_msg.len() > 12000 {
        crate::db::truncate_str(&assistant_msg, 12000).to_string()
    } else {
        assistant_msg
    };

    let mut conn = db::open_db()?;
    if db::is_summarize_on_cooldown(&conn, &project, SUMMARIZE_COOLDOWN_SECS)? {
        crate::log::info(
            "summary-job",
            &format!("project={} on cooldown, skipping", project),
        );
        return Ok(());
    }

    let msg_hash = hash_message(&msg);
    if db::is_duplicate_message(&conn, &project, &msg_hash)? {
        crate::log::info(
            "summary-job",
            &format!("project={} duplicate message, skipping", project),
        );
        return Ok(());
    }

    let memory_sid = db::upsert_session(&conn, &session_id, &project, None)?;
    let existing_ctx = match db::get_summary_by_session(&conn, &memory_sid, &project)? {
        Some(prev) => {
            let mut parts = Vec::new();
            if let Some(r) = &prev.request {
                parts.push(format!(
                    "<request>{}</request>",
                    memory_format::xml_escape_text(r)
                ));
            }
            if let Some(c) = &prev.completed {
                parts.push(format!(
                    "<completed>{}</completed>",
                    memory_format::xml_escape_text(c)
                ));
            }
            if let Some(d) = &prev.decisions {
                parts.push(format!(
                    "<decisions>{}</decisions>",
                    memory_format::xml_escape_text(d)
                ));
            }
            if let Some(l) = &prev.learned {
                parts.push(format!(
                    "<learned>{}</learned>",
                    memory_format::xml_escape_text(l)
                ));
            }
            if let Some(n) = &prev.next_steps {
                parts.push(format!(
                    "<next_steps>{}</next_steps>",
                    memory_format::xml_escape_text(n)
                ));
            }
            if let Some(p) = &prev.preferences {
                parts.push(format!(
                    "<preferences>{}</preferences>",
                    memory_format::xml_escape_text(p)
                ));
            }
            format!(
                "<existing_summary>\n{}\n</existing_summary>\n\n",
                parts.join("\n")
            )
        }
        None => String::new(),
    };
    let user_message = format!(
        "{}Here is the assistant's last response from the session:\n\n{}",
        existing_ctx, msg
    );

    if !db::try_acquire_summarize_lock(&mut conn, &project, SUMMARIZE_LOCK_TIMEOUT_SECS)? {
        crate::log::info(
            "summary-job",
            &format!("project={} summarize lock held, skipping", project),
        );
        return Ok(());
    }

    let ai_start = std::time::Instant::now();
    let response = match crate::ai::call_ai(
        SUMMARY_PROMPT,
        &user_message,
        crate::ai::UsageContext {
            project: Some(&project),
            operation: "summarize",
        },
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            let _ = db::release_summarize_lock(&conn, &project);
            anyhow::bail!("summary ai failed: {}", e);
        }
    };
    crate::log::info(
        "summary-job",
        &format!("AI response {}ms {}B", ai_start.elapsed().as_millis(), response.len()),
    );

    let Some(summary) = parse_summary(&response) else {
        let _ = db::release_summarize_lock(&conn, &project);
        crate::log::info("summary-job", "session skipped by AI");
        return Ok(());
    };

    let usage = response.len() as i64 / 4;
    let _deleted = db::finalize_summarize(
        &mut conn,
        &memory_sid,
        &project,
        &msg_hash,
        summary.request.as_deref(),
        summary.completed.as_deref(),
        summary.decisions.as_deref(),
        summary.learned.as_deref(),
        summary.next_steps.as_deref(),
        summary.preferences.as_deref(),
        None,
        usage,
    )?;
    db::release_summarize_lock(&conn, &project)?;
    crate::log::info(
        "summary-job",
        &format!("saved summary project={} session={}", project, session_id),
    );

    // Auto-promote summary fields to memories (zero LLM cost)
    if let Err(e) = crate::memory::promote_summary_to_memories(
        &conn,
        &session_id,
        &project,
        summary.request.as_deref(),
        summary.decisions.as_deref(),
        summary.learned.as_deref(),
        summary.preferences.as_deref(),
    ) {
        crate::log::warn("summary-job", &format!("memory promotion failed: {}", e));
    }

    // Sync to Claude Code native memory directory
    if let Err(e) = crate::claude_memory::sync_to_claude_memory(cwd, &project) {
        crate::log::warn("summary-job", &format!("claude memory sync failed: {}", e));
    }

    Ok(())
}

pub async fn process_compress_job(project: &str) -> Result<()> {
    maybe_compress(project).await
}

// --- Long-term memory compression ---

const COMPRESS_PROMPT: &str = include_str!("../prompts/compress.txt");
const COMPRESS_THRESHOLD: i64 = 100;
const KEEP_RECENT: i64 = 50;
const COMPRESS_BATCH: i64 = 30;

/// Compress old observations when count exceeds threshold.
/// Runs at the end of summarize_worker, in background.
async fn maybe_compress(project: &str) -> Result<()> {
    let conn = db::open_db()?;
    let total = db::count_active_observations(&conn, project)?;

    if total <= COMPRESS_THRESHOLD {
        return Ok(());
    }

    crate::log::info(
        "compress",
        &format!(
            "project={} has {} observations (threshold={}), compressing",
            project, total, COMPRESS_THRESHOLD
        ),
    );

    let old_obs = db::get_oldest_observations(&conn, project, KEEP_RECENT, COMPRESS_BATCH)?;
    if old_obs.is_empty() {
        return Ok(());
    }

    let timer = crate::log::Timer::start("compress", &format!("{} observations", old_obs.len()));

    // Build input for AI
    let mut events = String::from("<old_observations>\n");
    for obs in &old_obs {
        events.push_str(&format!(
            "<observation type=\"{}\">\n<title>{}</title>\n<subtitle>{}</subtitle>\n<narrative>{}</narrative>\n</observation>\n",
            memory_format::xml_escape_attr(&obs.r#type),
            memory_format::xml_escape_text(obs.title.as_deref().unwrap_or("")),
            memory_format::xml_escape_text(obs.subtitle.as_deref().unwrap_or("")),
            memory_format::xml_escape_text(obs.narrative.as_deref().unwrap_or("")),
        ));
    }
    events.push_str("</old_observations>");

    let response = match crate::ai::call_ai(
        COMPRESS_PROMPT,
        &events,
        crate::ai::UsageContext {
            project: Some(project),
            operation: "compress",
        },
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            crate::log::warn("compress", &format!("AI call failed: {}", e));
            timer.done(&format!("AI error: {}", e));
            return Ok(());
        }
    };

    let compressed = memory_format::parse_observations(&response);

    // Store compressed observations (if any)
    if !compressed.is_empty() {
        let memory_session_id = format!("compressed-{}", chrono::Utc::now().timestamp());
        let usage = response.len() as i64 / 4;

        for obs in &compressed {
            let facts_json = if obs.facts.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&obs.facts)?)
            };
            let concepts_json = if obs.concepts.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&obs.concepts)?)
            };

            db::insert_observation(
                &conn,
                &memory_session_id,
                project,
                &obs.obs_type,
                obs.title.as_deref(),
                obs.subtitle.as_deref(),
                obs.narrative.as_deref(),
                facts_json.as_deref(),
                concepts_json.as_deref(),
                None,
                None,
                None,
                usage / compressed.len().max(1) as i64,
            )?;
        }
    }

    // Mark old observations as compressed
    let ids: Vec<i64> = old_obs.iter().map(|o| o.id).collect();
    let marked = db::mark_observations_compressed(&conn, &ids)?;

    timer.done(&format!(
        "{} old → {} compressed, {} marked",
        old_obs.len(),
        compressed.len(),
        marked
    ));

    Ok(())
}

