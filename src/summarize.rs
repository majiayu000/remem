use anyhow::Result;
use serde::Deserialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::db;
use crate::observe;

const SUMMARY_PROMPT: &str = include_str!("../prompts/summary.txt");

/// 同项目 summarize 冷却期（秒）
const SUMMARIZE_COOLDOWN_SECS: i64 = 300;

/// 触发 summarize 的最小 pending 数量
const MIN_PENDING_FOR_SUMMARIZE: i64 = 3;

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
        let val: serde_json::Value = serde_json::from_str(line).ok()?;
        if val["type"].as_str() == Some("assistant") {
            let text_parts: Vec<&str> = val["message"]["content"]
                .as_array()?
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

    let start = text.find("<summary>")?;
    let end = text.find("</summary>")?;
    let content = &text[start + "<summary>".len()..end];

    Some(ParsedSummary {
        request: observe::extract_field(content, "request"),
        completed: observe::extract_field(content, "completed"),
        decisions: observe::extract_field(content, "decisions"),
        learned: observe::extract_field(content, "learned"),
        next_steps: observe::extract_field(content, "next_steps"),
        preferences: observe::extract_field(content, "preferences"),
    })
}

/// Stop hook entry: read stdin, spawn background worker, return immediately.
/// 多层 gate 防止 token 浪费：
/// 1. pending >= MIN_PENDING_FOR_SUMMARIZE（过滤短命 session）
/// 2. 项目冷却期（同项目 5 分钟内不重复 summarize）
/// 3. message hash 去重（相同 assistant message 不重复处理）
pub async fn summarize() -> Result<()> {
    let input = std::io::read_to_string(std::io::stdin())?;

    // Quick validation
    let hook: SummarizeInput = serde_json::from_str(&input)?;
    let Some(session_id) = &hook.session_id else {
        return Ok(());
    };
    if let Some(msg) = &hook.last_assistant_message {
        if msg.contains("<skip_summary") || msg.len() < 50 {
            return Ok(());
        }
    }

    let conn = db::open_db()?;

    // Gate 1: 最小 pending 数量（过滤短命 session 和无操作 session）
    let pending = db::count_pending(&conn, session_id)?;
    if pending < MIN_PENDING_FOR_SUMMARIZE {
        crate::log::info("summarize", &format!(
            "session={} has {} pending (min={}), skipping", session_id, pending, MIN_PENDING_FOR_SUMMARIZE
        ));
        return Ok(());
    }

    let cwd = hook.cwd.as_deref().unwrap_or(".");
    let project = db::project_from_cwd(cwd);

    // Gate 2: 项目冷却期
    if db::is_summarize_on_cooldown(&conn, &project, SUMMARIZE_COOLDOWN_SECS)? {
        crate::log::info("summarize", &format!(
            "project={} on cooldown ({}s), skipping", project, SUMMARIZE_COOLDOWN_SECS
        ));
        return Ok(());
    }

    // Gate 3: message hash 去重
    if let Some(msg) = &hook.last_assistant_message {
        let msg_hash = hash_message(msg);
        if db::is_duplicate_message(&conn, &project, &msg_hash)? {
            crate::log::info("summarize", &format!(
                "project={} duplicate message hash, skipping", project
            ));
            return Ok(());
        }
    }

    crate::log::info("summarize", &format!(
        "session={} project={} pending={}, dispatching worker", session_id, project, pending
    ));

    // Spawn background worker — Stop hook returns immediately
    let exe = std::env::current_exe()?;
    let mut child = std::process::Command::new(&exe)
        .arg("summarize-worker")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(input.as_bytes())?;
    }

    crate::log::info("summarize", "dispatched to background worker");
    Ok(())
}

/// Max total runtime for a single worker invocation (seconds).
/// Guards against hangs in AI calls or DB operations.
const WORKER_TIMEOUT_SECS: u64 = 180;

/// Background worker: does the actual AI calls. Runs detached from Claude Code.
pub async fn summarize_worker() -> Result<()> {
    // Global timeout: kill entire worker if it hangs
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(WORKER_TIMEOUT_SECS),
        summarize_worker_inner(),
    )
    .await;
    match result {
        Ok(inner) => inner,
        Err(_) => {
            crate::log::warn("summarize-worker", &format!(
                "worker timed out after {}s", WORKER_TIMEOUT_SECS
            ));
            Ok(())
        }
    }
}

async fn summarize_worker_inner() -> Result<()> {
    let timer = crate::log::Timer::start("summarize-worker", "");
    let input = std::io::read_to_string(std::io::stdin())?;
    crate::log::debug("summarize-worker", &format!("input: {}", crate::db::truncate_str(&input, 500)));

    let hook: SummarizeInput = serde_json::from_str(&input)?;

    let Some(session_id) = hook.session_id else {
        timer.done("skipped (no session_id)");
        return Ok(());
    };
    let cwd = hook.cwd.as_deref().unwrap_or(".");
    let project = project_from_cwd(cwd);

    crate::log::info("summarize-worker", &format!("project={} session={}", project, session_id));

    // Flush pending observation queue (current session)
    match observe::flush_pending(&session_id, &project).await {
        Ok(n) => {
            if n > 0 {
                crate::log::info("summarize-worker", &format!("flushed {} observations", n));
            }
        }
        Err(e) => {
            crate::log::warn("summarize-worker", &format!("flush failed (continuing): {}", e));
        }
    }

    // Flush stale pending from other sessions in same project (>10 min old).
    // This runs in the background worker where AI calls are safe.
    {
        let conn = db::open_db()?;
        match db::get_stale_pending_sessions(&conn, &project, 600) {
            Ok(stale_sessions) => {
                for sid in &stale_sessions {
                    if *sid == session_id {
                        continue;
                    }
                    match observe::flush_pending(sid, &project).await {
                        Ok(n) if n > 0 => {
                            crate::log::info("summarize-worker", &format!(
                                "auto-flushed {} stale pending from session={}", n, sid
                            ));
                        }
                        Err(e) => {
                            crate::log::warn("summarize-worker", &format!(
                                "stale flush failed session={}: {}", sid, e
                            ));
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                crate::log::warn("summarize-worker", &format!("stale pending query failed: {}", e));
            }
        }
    }

    // Trigger compression if needed (after flush, independent of summary success)
    if let Err(e) = maybe_compress(&project).await {
        crate::log::warn("summarize-worker", &format!("compress failed: {}", e));
    }

    // Get last assistant message
    let assistant_msg = hook
        .last_assistant_message
        .or_else(|| {
            hook.transcript_path
                .as_deref()
                .and_then(extract_last_assistant_message)
        })
        .unwrap_or_default();

    if assistant_msg.is_empty() {
        timer.done("no message");
        return Ok(());
    }

    if assistant_msg.contains("<skip_summary") || assistant_msg.len() < 50 {
        timer.done("skipped (trivial)");
        return Ok(());
    }

    crate::log::info("summarize-worker", &format!("message len={}B", assistant_msg.len()));

    // Truncate if too long
    let msg = if assistant_msg.len() > 12000 {
        crate::db::truncate_str(&assistant_msg, 12000).to_string()
    } else {
        assistant_msg
    };

    // Gate: worker 端再次检查冷却期（防止多个 worker 并行）
    let conn_for_summary = db::open_db()?;
    if db::is_summarize_on_cooldown(&conn_for_summary, &project, SUMMARIZE_COOLDOWN_SECS)? {
        crate::log::info("summarize-worker", &format!(
            "project={} on cooldown, skipping AI call", project
        ));
        timer.done("skipped (cooldown)");
        return Ok(());
    }

    // Gate: message hash 去重（worker 端双重检查）
    let msg_hash = hash_message(&msg);
    if db::is_duplicate_message(&conn_for_summary, &project, &msg_hash)? {
        crate::log::info("summarize-worker", &format!(
            "project={} duplicate message, skipping AI call", project
        ));
        timer.done("skipped (duplicate message)");
        return Ok(());
    }

    // 提前记录冷却期（防止并行 worker 同时通过 gate）
    db::record_summarize(&conn_for_summary, &project, &msg_hash)?;
    let memory_sid = db::upsert_session(&conn_for_summary, &session_id, &project, None)?;
    let existing_ctx = match db::get_summary_by_session(&conn_for_summary, &memory_sid, &project)? {
        Some(prev) => {
            let mut parts = Vec::new();
            if let Some(r) = &prev.request { parts.push(format!("<request>{}</request>", r)); }
            if let Some(c) = &prev.completed { parts.push(format!("<completed>{}</completed>", c)); }
            if let Some(d) = &prev.decisions { parts.push(format!("<decisions>{}</decisions>", d)); }
            if let Some(l) = &prev.learned { parts.push(format!("<learned>{}</learned>", l)); }
            if let Some(n) = &prev.next_steps { parts.push(format!("<next_steps>{}</next_steps>", n)); }
            if let Some(p) = &prev.preferences { parts.push(format!("<preferences>{}</preferences>", p)); }
            format!("<existing_summary>\n{}\n</existing_summary>\n\n", parts.join("\n"))
        }
        None => String::new(),
    };

    let user_message = format!(
        "{}Here is the assistant's last response from the session:\n\n{}",
        existing_ctx, msg
    );

    let ai_start = std::time::Instant::now();
    let response = match observe::call_anthropic(SUMMARY_PROMPT, &user_message).await {
        Ok(r) => r,
        Err(e) => {
            crate::log::warn("summarize-worker", &format!("AI call failed: {}", e));
            timer.done(&format!("AI error: {}", e));
            return Ok(());
        }
    };
    let ai_ms = ai_start.elapsed().as_millis();
    crate::log::info("summarize-worker", &format!("AI response {}ms {}B", ai_ms, response.len()));

    let Some(summary) = parse_summary(&response) else {
        crate::log::info("summarize-worker", "session skipped by AI (trivial)");
        timer.done("skipped");
        return Ok(());
    };

    // Delete previous summary for this session (replaced by the new merged one)
    let deleted = db::delete_summaries_by_session(&conn_for_summary, &memory_sid, &project)?;
    if deleted > 0 {
        crate::log::info("summarize-worker", &format!("replaced {} old summary(s)", deleted));
    }

    let usage = response.len() as i64 / 4;
    db::insert_summary(
        &conn_for_summary,
        &memory_sid,
        &project,
        summary.request.as_deref(),
        summary.completed.as_deref(),
        summary.decisions.as_deref(),
        summary.learned.as_deref(),
        summary.next_steps.as_deref(),
        summary.preferences.as_deref(),
        None,
        usage,
    )?;

    let request_preview = summary.request.as_deref().unwrap_or("-");
    timer.done(&format!("~{}tok request=\"{}\"", usage, request_preview));
    Ok(())
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
        &format!("project={} has {} observations (threshold={}), compressing", project, total, COMPRESS_THRESHOLD),
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
            obs.r#type,
            obs.title.as_deref().unwrap_or(""),
            obs.subtitle.as_deref().unwrap_or(""),
            obs.narrative.as_deref().unwrap_or(""),
        ));
    }
    events.push_str("</old_observations>");

    let response = match observe::call_anthropic(COMPRESS_PROMPT, &events).await {
        Ok(r) => r,
        Err(e) => {
            crate::log::warn("compress", &format!("AI call failed: {}", e));
            timer.done(&format!("AI error: {}", e));
            return Ok(());
        }
    };

    let compressed = observe::parse_observations(&response);

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
