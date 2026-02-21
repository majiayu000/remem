use anyhow::Result;
use serde::Deserialize;

use crate::db;

const OBSERVATION_PROMPT: &str = include_str!("../prompts/observation.txt");

const VALID_TYPES: &[&str] = &[
    "bugfix",
    "feature",
    "refactor",
    "change",
    "discovery",
    "decision",
];

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

pub async fn call_anthropic(
    system: &str,
    user_message: &str,
    project: &str,
    operation: &str,
) -> Result<String> {
    crate::ai::call_ai(
        system,
        user_message,
        crate::ai::UsageContext {
            project: Some(project),
            operation,
        },
    )
    .await
}

pub struct ParsedObservation {
    pub obs_type: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub facts: Vec<String>,
    pub narrative: Option<String>,
    pub concepts: Vec<String>,
    pub files_read: Vec<String>,
    pub files_modified: Vec<String>,
}

pub fn extract_field(content: &str, field: &str) -> Option<String> {
    let open = format!("<{}>", field);
    let close = format!("</{}>", field);
    let start = content.find(&open)? + open.len();
    let end = content.find(&close)?;
    if start >= end {
        return None;
    }
    let val = content[start..end].trim().to_string();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

fn extract_array(content: &str, array_name: &str, element_name: &str) -> Vec<String> {
    let open = format!("<{}>", array_name);
    let close = format!("</{}>", array_name);
    let Some(start) = content.find(&open) else {
        return vec![];
    };
    let Some(end) = content.find(&close) else {
        return vec![];
    };
    let inner = &content[start + open.len()..end];

    let elem_open = format!("<{}>", element_name);
    let elem_close = format!("</{}>", element_name);
    let mut results = Vec::new();
    let mut pos = 0;
    while let Some(s) = inner[pos..].find(&elem_open) {
        let val_start = pos + s + elem_open.len();
        if let Some(e) = inner[val_start..].find(&elem_close) {
            let val = inner[val_start..val_start + e].trim().to_string();
            if !val.is_empty() {
                results.push(val);
            }
            pos = val_start + e + elem_close.len();
        } else {
            break;
        }
    }
    results
}

pub fn parse_observations(text: &str) -> Vec<ParsedObservation> {
    let mut observations = Vec::new();
    let mut pos = 0;
    while let Some(start) = text[pos..].find("<observation>") {
        let obs_start = pos + start + "<observation>".len();
        if let Some(end) = text[obs_start..].find("</observation>") {
            let content = &text[obs_start..obs_start + end];

            let raw_type = extract_field(content, "type").unwrap_or_default();
            let obs_type = if VALID_TYPES.contains(&raw_type.as_str()) {
                raw_type
            } else {
                "discovery".to_string()
            };

            let mut concepts = extract_array(content, "concepts", "concept");
            concepts.retain(|c| c != &obs_type);

            observations.push(ParsedObservation {
                obs_type,
                title: extract_field(content, "title"),
                subtitle: extract_field(content, "subtitle"),
                facts: extract_array(content, "facts", "fact"),
                narrative: extract_field(content, "narrative"),
                concepts,
                files_read: extract_array(content, "files_read", "file"),
                files_modified: extract_array(content, "files_modified", "file"),
            });

            pos = obs_start + end + "</observation>".len();
        } else {
            break;
        }
    }
    observations
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
            let cmd_trimmed = cmd.trim();
            if BASH_SKIP_PREFIXES
                .iter()
                .any(|prefix| cmd_trimmed.starts_with(prefix))
            {
                crate::log::info(
                    "observe",
                    &format!("SKIP bash cmd={}", db::truncate_str(cmd_trimmed, 60)),
                );
                return Ok(());
            }
        }
    }

    let tool_input_str = hook
        .tool_input
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_default());
    let tool_response_str = hook.tool_response.as_ref().map(|v| {
        let s = serde_json::to_string(v).unwrap_or_default();
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
    let all_types: &[&str] = &[
        "bugfix",
        "feature",
        "refactor",
        "change",
        "discovery",
        "decision",
    ];
    let recent = db::query_observations(conn, project, all_types, 10)?;
    if recent.is_empty() {
        return Ok(String::new());
    }

    let mut buf = String::from("<existing_memories>\n");
    for obs in &recent {
        buf.push_str(&format!(
            "<memory type=\"{}\">{}{}</memory>\n",
            obs.r#type,
            obs.title
                .as_deref()
                .map(|t| format!(" title=\"{}\"", t))
                .unwrap_or_default(),
            obs.subtitle
                .as_deref()
                .map(|s| format!(" — {}", s))
                .unwrap_or_default(),
        ));
    }
    buf.push_str("</existing_memories>\n");
    Ok(buf)
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

    // Build batch prompt with all events
    let mut events = String::new();
    for (i, p) in pending.iter().enumerate() {
        events.push_str(&format!(
            "<event index=\"{}\">\n\
             <tool>{}</tool>\n\
             <working_directory>{}</working_directory>\n\
             <parameters>{}</parameters>\n\
             <outcome>{}</outcome>\n\
             </event>\n",
            i + 1,
            p.tool_name,
            p.cwd.as_deref().unwrap_or("."),
            p.tool_input.as_deref().unwrap_or(""),
            p.tool_response.as_deref().unwrap_or(""),
        ));
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

    let user_message = format!(
        "{}<session_events>\n{}</session_events>",
        existing_context, events
    );

    // Single AI call for all events
    let ai_start = std::time::Instant::now();
    let response = match call_anthropic(OBSERVATION_PROMPT, &user_message, project, "flush").await {
        Ok(r) => r,
        Err(e) => {
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
        &format!("AI response {}ms {}B", ai_ms, response.len()),
    );

    // Parse and store observations
    let observations = parse_observations(&response);
    if observations.is_empty() {
        crate::log::info("flush", "no observations extracted from batch");
        let ids: Vec<i64> = pending.iter().map(|p| p.id).collect();
        db::delete_pending_claimed(&conn, &lease_owner, &ids)?;
        timer.done("0 observations");
        return Ok(0);
    }

    let usage = response.len() as i64 / 4;
    let ids: Vec<i64> = pending.iter().map(|p| p.id).collect();
    let persist_result: Result<()> = (|| {
        let tx = conn.transaction()?;
        let memory_session_id = db::upsert_session(&tx, session_id, project, None)?;

        for obs in &observations {
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
                usage / observations.len().max(1) as i64,
            )?;

            if !obs.files_modified.is_empty() {
                let stale_count =
                    db::mark_stale_by_files(&tx, obs_id, project, &obs.files_modified)?;
                if stale_count > 0 {
                    crate::log::info(
                        "flush",
                        &format!("marked {} stale (file overlap)", stale_count),
                    );
                }
            }
        }

        let deleted = db::delete_pending_claimed(&tx, &lease_owner, &ids)?;
        if deleted != ids.len() {
            anyhow::bail!(
                "pending ack mismatch: expected {}, deleted {}",
                ids.len(),
                deleted
            );
        }

        tx.commit()?;
        Ok(())
    })();
    if let Err(e) = persist_result {
        if let Err(release_err) = db::release_pending_claims(&conn, &lease_owner) {
            crate::log::warn("flush", &format!("release claim failed: {}", release_err));
        }
        return Err(e);
    }

    let titles: Vec<&str> = observations
        .iter()
        .filter_map(|o| o.title.as_deref())
        .collect();
    timer.done(&format!(
        "{} events → {} observations (~{}tok) [{}]",
        pending.len(),
        observations.len(),
        usage,
        titles.join(", ")
    ));

    Ok(observations.len())
}
