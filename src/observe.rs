use anyhow::Result;
use serde::Deserialize;

use crate::db;
use crate::memory_format::{self, xml_escape_attr, xml_escape_text, OBSERVATION_TYPES};

const OBSERVATION_PROMPT: &str = include_str!("../prompts/observation.txt");

/// Tools that produce meaningful observations (modify state)
const ACTION_TOOLS: &[&str] = &["Write", "Edit", "NotebookEdit", "Bash"];

/// Tools to always skip (metadata/navigation)
const SKIP_TOOLS: &[&str] = &[
    "ListMcpResourcesTool",
    "SlashCommand",
    "Skill",
    "TodoWrite",
    "AskUserQuestion",
    "TaskCreate",
    "TaskUpdate",
    "TaskList",
    "TaskGet",
    "EnterPlanMode",
    "ExitPlanMode",
];

/// Bash command prefixes to skip (routine/read-only operations, not worth recording)
const BASH_SKIP_PREFIXES: &[&str] = &[
    "git status",
    "git log",
    "git diff",
    "git branch",
    "git stash list",
    "git remote",
    "git fetch",
    "git show",
    "ls",
    "pwd",
    "echo ",
    "which ",
    "type ",
    "whereis ",
    "cat ",
    "head ",
    "tail ",
    "wc ",
    "file ",
    "npm install",
    "npm ci",
    "yarn install",
    "pnpm install",
    "cargo build",
    "cargo check",
    "cargo clippy",
    "cargo fmt",
    "cd ",
    "pushd ",
    "popd",
    "lsof ",
    "ps ",
    "top",
    "htop",
    "df ",
    "du ",
];

/// Max tool_response size stored in queue (save DB space)
const MAX_RESPONSE_SIZE: usize = 4000;

/// Max events per flush batch (prevents oversized AI input)
const FLUSH_BATCH_SIZE: usize = 15;
/// On AI timeout, split large batches recursively to improve success rate.
const FLUSH_RETRY_MIN_BATCH_SIZE: usize = 1;
/// Pending lease duration for a single flush worker.
const PENDING_LEASE_SECS: i64 = 240;

#[derive(Debug, Deserialize)]
struct HookInput {
    session_id: Option<String>,
    cwd: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    tool_response: Option<serde_json::Value>,
}

use crate::db::project_from_cwd;

pub fn should_skip_bash_command(cmd: &str) -> bool {
    let cmd_trimmed = cmd.trim();
    BASH_SKIP_PREFIXES
        .iter()
        .any(|prefix| cmd_trimmed.starts_with(prefix))
}

pub async fn session_init() -> Result<()> {
    let timer = crate::log::Timer::start("session-init", "");
    let input = std::io::read_to_string(std::io::stdin())?;
    crate::log::debug(
        "session-init",
        &format!("raw input: {}", crate::db::truncate_str(&input, 500)),
    );
    let hook: HookInput = serde_json::from_str(&input)?;

    let session_id = hook
        .session_id
        .ok_or_else(|| anyhow::anyhow!("missing session_id"))?;
    let cwd = hook.cwd.as_deref().unwrap_or(".");
    let project = project_from_cwd(cwd);

    crate::log::info(
        "session-init",
        &format!("project={} session={}", project, session_id),
    );

    let conn = db::open_db()?;
    db::upsert_session(&conn, &session_id, &project, None)?;

    // Lightweight cleanup only — no AI calls in hook context.
    // Stale flush (with AI) is handled by summarize_worker instead.
    let stale = db::cleanup_stale_pending(&conn)?;
    if stale > 0 {
        crate::log::info(
            "session-init",
            &format!("cleaned {} stale pending (>1h)", stale),
        );
    }

    timer.done(&format!("project={}", project));
    Ok(())
}

/// PostToolUse hook: queue to SQLite, no AI call.
pub async fn observe() -> Result<()> {
    let input = std::io::read_to_string(std::io::stdin())?;
    let hook: HookInput = serde_json::from_str(&input)?;

    let session_id = hook
        .session_id
        .ok_or_else(|| anyhow::anyhow!("missing session_id"))?;
    let cwd = hook.cwd.as_deref().unwrap_or(".");
    let project = project_from_cwd(cwd);
    let tool_name = hook.tool_name.as_deref().unwrap_or("unknown");

    // Skip metadata tools
    if SKIP_TOOLS.contains(&tool_name) {
        return Ok(());
    }

    // Only queue action tools (Write, Edit, Bash, NotebookEdit)
    if !ACTION_TOOLS.contains(&tool_name) {
        crate::log::info("observe", &format!("SKIP tool={} (read-only)", tool_name));
        return Ok(());
    }

    // Filter out routine Bash commands (read-only/build operations)
    if tool_name == "Bash" {
        if let Some(cmd) = hook.tool_input.as_ref().and_then(|v| v["command"].as_str()) {
            if should_skip_bash_command(cmd) {
                crate::log::info(
                    "observe",
                    &format!("SKIP bash cmd={}", db::truncate_str(cmd.trim(), 60)),
                );
                return Ok(());
            }
        }
    }

    let tool_input_str = hook.tool_input.as_ref().map(|v| {
        serde_json::to_string(v).unwrap_or_else(|e| {
            crate::log::warn("observe", &format!("tool_input serialize failed: {}", e));
            "{}".to_string()
        })
    });
    let tool_response_str = hook.tool_response.as_ref().map(|v| {
        let s = serde_json::to_string(v).unwrap_or_else(|e| {
            crate::log::warn("observe", &format!("tool_response serialize failed: {}", e));
            "{}".to_string()
        });
        if s.len() > MAX_RESPONSE_SIZE {
            crate::db::truncate_str(&s, MAX_RESPONSE_SIZE).to_string()
        } else {
            s
        }
    });

    let conn = db::open_db()?;
    db::enqueue_pending(
        &conn,
        &session_id,
        &project,
        tool_name,
        tool_input_str.as_deref(),
        tool_response_str.as_deref(),
        Some(cwd),
    )?;

    let count = db::count_pending(&conn, &session_id)?;
    crate::log::info(
        "observe",
        &format!(
            "QUEUED tool={} project={} pending={}",
            tool_name, project, count
        ),
    );

    Ok(())
}

/// Build existing memory context for delta deduplication.
/// Returns formatted XML block, or empty string if no recent memories.
fn build_existing_context(conn: &rusqlite::Connection, project: &str) -> Result<String> {
    let recent = db::query_observations(conn, project, OBSERVATION_TYPES, 10)?;
    if recent.is_empty() {
        return Ok(String::new());
    }

    let mut buf = String::from("<existing_memories>\n");
    for obs in &recent {
        buf.push_str(&format!(
            "<memory type=\"{}\">{}{}</memory>\n",
            xml_escape_attr(&obs.r#type),
            obs.title
                .as_deref()
                .map(|t| format!(" title=\"{}\"", xml_escape_attr(t)))
                .unwrap_or_default(),
            obs.subtitle
                .as_deref()
                .map(|s| format!(" — {}", xml_escape_text(s)))
                .unwrap_or_default(),
        ));
    }
    buf.push_str("</existing_memories>\n");
    Ok(buf)
}

fn build_session_events_xml(batch: &[db::PendingObservation]) -> String {
    let mut events = String::new();
    for (i, p) in batch.iter().enumerate() {
        events.push_str(&format!(
            "<event index=\"{}\">\n\
             <tool>{}</tool>\n\
             <working_directory>{}</working_directory>\n\
             <parameters>{}</parameters>\n\
             <outcome>{}</outcome>\n\
             </event>\n",
            i + 1,
            xml_escape_text(&p.tool_name),
            xml_escape_text(p.cwd.as_deref().unwrap_or(".")),
            xml_escape_text(p.tool_input.as_deref().unwrap_or("")),
            xml_escape_text(p.tool_response.as_deref().unwrap_or("")),
        ));
    }
    events
}

fn is_ai_timeout_error(err: &anyhow::Error) -> bool {
    err.to_string().to_lowercase().contains("timed out")
}

fn persist_flush_batch(
    conn: &mut rusqlite::Connection,
    session_id: &str,
    project: &str,
    lease_owner: &str,
    batch: &[db::PendingObservation],
    observations: &[memory_format::ParsedObservation],
    usage: i64,
) -> Result<()> {
    let ids: Vec<i64> = batch.iter().map(|p| p.id).collect();
    let per_obs_usage = usage / observations.len().max(1) as i64;

    let tx = conn.transaction()?;
    let memory_session_id = db::upsert_session(&tx, session_id, project, None)?;

    for obs in observations {
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
        let files_read_json = if obs.files_read.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&obs.files_read)?)
        };
        let files_modified_json = if obs.files_modified.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&obs.files_modified)?)
        };

        let obs_id = db::insert_observation(
            &tx,
            &memory_session_id,
            project,
            &obs.obs_type,
            obs.title.as_deref(),
            obs.subtitle.as_deref(),
            obs.narrative.as_deref(),
            facts_json.as_deref(),
            concepts_json.as_deref(),
            files_read_json.as_deref(),
            files_modified_json.as_deref(),
            None,
            per_obs_usage,
        )?;

        if !obs.files_modified.is_empty() {
            let stale_count = db::mark_stale_by_files(&tx, obs_id, project, &obs.files_modified)?;
            if stale_count > 0 {
                crate::log::info(
                    "flush",
                    &format!("marked {} stale (file overlap)", stale_count),
                );
            }
        }
    }

    let deleted = db::delete_pending_claimed(&tx, lease_owner, &ids)?;
    if deleted != ids.len() {
        anyhow::bail!(
            "pending ack mismatch: expected {}, deleted {}",
            ids.len(),
            deleted
        );
    }

    tx.commit()?;
    Ok(())
}

/// Flush pending queue: batch all queued items into one AI call.
pub async fn flush_pending(session_id: &str, project: &str) -> Result<usize> {
    let mut conn = db::open_db()?;
    let lease_owner = format!(
        "flush-{}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis(),
        crate::db::truncate_str(session_id, 8)
    );
    let pending = db::claim_pending(
        &conn,
        session_id,
        FLUSH_BATCH_SIZE,
        &lease_owner,
        PENDING_LEASE_SECS,
    )?;

    if pending.is_empty() {
        crate::log::info("flush", "no pending observations");
        return Ok(0);
    }

    let timer = crate::log::Timer::start(
        "flush",
        &format!("{} events project={}", pending.len(), project),
    );

    // Large batches may timeout in claude CLI; split and retry recursively.
    let mut ranges: Vec<(usize, usize)> = vec![(0, pending.len())];
    let mut total_observations = 0usize;
    let mut total_usage = 0i64;
    let mut split_retries = 0usize;
    let mut titles: Vec<String> = Vec::new();

    while let Some((start, end)) = ranges.pop() {
        let batch = &pending[start..end];
        if batch.is_empty() {
            continue;
        }

        // Delta: include recent existing memories so AI skips duplicates
        let existing_context = match build_existing_context(&conn, project) {
            Ok(ctx) => ctx,
            Err(e) => {
                crate::log::warn(
                    "flush",
                    &format!("existing context failed (continuing): {}", e),
                );
                String::new()
            }
        };

        let events = build_session_events_xml(batch);
        let user_message = format!(
            "{}<session_events>\n{}</session_events>",
            existing_context, events
        );

        let ai_start = std::time::Instant::now();
        let response = match crate::ai::call_ai(
            OBSERVATION_PROMPT,
            &user_message,
            crate::ai::UsageContext {
                project: Some(project),
                operation: "flush",
            },
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let can_split = is_ai_timeout_error(&e) && batch.len() > FLUSH_RETRY_MIN_BATCH_SIZE;
                if can_split {
                    let mid = start + (batch.len() / 2);
                    if mid > start && mid < end {
                        split_retries += 1;
                        crate::log::warn(
                            "flush",
                            &format!(
                                "AI timeout on {} events, splitting into {} + {}",
                                batch.len(),
                                mid - start,
                                end - mid
                            ),
                        );
                        // DFS order: process left half first.
                        ranges.push((mid, end));
                        ranges.push((start, mid));
                        continue;
                    }
                }

                if let Err(release_err) = db::release_pending_claims(&conn, &lease_owner) {
                    crate::log::warn("flush", &format!("release claim failed: {}", release_err));
                }
                crate::log::warn("flush", &format!("AI call failed: {}", e));
                timer.done(&format!("AI error: {}", e));
                return Err(e);
            }
        };
        let ai_ms = ai_start.elapsed().as_millis();
        crate::log::info(
            "flush",
            &format!(
                "AI response {}ms {}B (batch {} events)",
                ai_ms,
                response.len(),
                batch.len()
            ),
        );

        let observations = memory_format::parse_observations(&response);
        if observations.is_empty() {
            crate::log::info(
                "flush",
                &format!(
                    "no observations extracted from batch ({} events)",
                    batch.len()
                ),
            );
            let ids: Vec<i64> = batch.iter().map(|p| p.id).collect();
            db::delete_pending_claimed(&conn, &lease_owner, &ids)?;
            continue;
        }

        let usage = response.len() as i64 / 4;
        if let Err(e) = persist_flush_batch(
            &mut conn,
            session_id,
            project,
            &lease_owner,
            batch,
            &observations,
            usage,
        ) {
            if let Err(release_err) = db::release_pending_claims(&conn, &lease_owner) {
                crate::log::warn("flush", &format!("release claim failed: {}", release_err));
            }
            return Err(e);
        }

        total_usage += usage;
        total_observations += observations.len();
        titles.extend(
            observations
                .iter()
                .filter_map(|o| o.title.as_deref().map(str::to_string)),
        );
    }

    if total_observations == 0 {
        timer.done("0 observations");
        return Ok(0);
    }

    let retry_suffix = if split_retries > 0 {
        format!("; split_retries={}", split_retries)
    } else {
        String::new()
    };
    timer.done(&format!(
        "{} events → {} observations (~{}tok) [{}]{}",
        pending.len(),
        total_observations,
        total_usage,
        titles.join(", "),
        retry_suffix
    ));

    Ok(total_observations)
}
