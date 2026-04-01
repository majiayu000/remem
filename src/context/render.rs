use anyhow::Result;

use crate::db;
use crate::db::project_from_cwd;

use super::format::format_header_datetime;
use super::query::load_context_data;
use super::sections::{
    render_core_memory, render_empty_state, render_memory_index, render_recent_sessions,
    render_workstreams,
};

pub fn generate_context(cwd: &str, _session_id: Option<&str>, _use_colors: bool) -> Result<()> {
    let timer = crate::log::Timer::start("context", &format!("cwd={}", cwd));
    let project = project_from_cwd(cwd);
    let current_branch = db::detect_git_branch(cwd);

    let conn = match db::open_db() {
        Ok(connection) => connection,
        Err(error) => {
            crate::log::warn(
                "context",
                &format!("open_db failed for project={}: {}", project, error),
            );
            render_empty_state(&project);
            timer.done("empty (no DB)");
            return Ok(());
        }
    };

    let loaded = load_context_data(&conn, &project, current_branch.as_deref());
    if loaded.memories.is_empty() && loaded.summaries.is_empty() && loaded.workstreams.is_empty() {
        render_empty_state(&project);
        timer.done("empty (no data)");
        return Ok(());
    }

    let mut output = String::new();
    output.push_str(&build_context_header(&project, current_branch.as_deref()));
    output.push_str(
        "Use `search`/`get_observations` for details. `save_memory` after decisions/bugfixes.\n\n",
    );

    if let Err(error) = crate::preference::render_preferences(&mut output, &conn, &project, cwd) {
        crate::log::warn("context", &format!("render_preferences failed: {}", error));
    }

    if !loaded.memories.is_empty() {
        render_core_memory(&mut output, &loaded.memories);
        render_memory_index(&mut output, &loaded.memories);
    }
    if !loaded.workstreams.is_empty() {
        render_workstreams(&mut output, &loaded.workstreams);
    }
    if !loaded.summaries.is_empty() {
        render_recent_sessions(&mut output, &loaded.summaries);
    }

    output.push_str(&format!("{} memories loaded.\n", loaded.memories.len()));
    print!("{}", output);

    timer.done(&format!(
        "project={} memories={} summaries={} workstreams={}",
        project,
        loaded.memories.len(),
        loaded.summaries.len(),
        loaded.workstreams.len(),
    ));
    Ok(())
}

fn build_context_header(project: &str, current_branch: Option<&str>) -> String {
    let branch_label = current_branch
        .map(|branch| format!(" @{}", branch))
        .unwrap_or_default();
    format!(
        "# [{}{}] context {}\n",
        project,
        branch_label,
        format_header_datetime()
    )
}
