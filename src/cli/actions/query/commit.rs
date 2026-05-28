use anyhow::Result;

use crate::cli::types::CommitAction;
use crate::{db, git_trace};

pub(in crate::cli) fn run_commit(action: CommitAction) -> Result<()> {
    let conn = db::open_db()?;
    match action {
        CommitAction::Show { sha, project, json } => {
            let results = git_trace::lookup_commit(&conn, project.as_deref(), &sha)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&results)?);
                return Ok(());
            }
            if results.is_empty() {
                println!("No commit metadata found for {sha}");
                return Ok(());
            }
            for (index, result) in results.iter().enumerate() {
                if index > 0 {
                    println!();
                }
                print_commit_lookup(result);
            }
        }
        CommitAction::Session {
            session_id,
            project,
            limit,
            json,
        } => {
            let results =
                git_trace::commits_for_session(&conn, project.as_deref(), &session_id, limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&results)?);
                return Ok(());
            }
            if results.is_empty() {
                println!("No commits linked to session {session_id}");
                return Ok(());
            }
            println!("Session: {session_id}");
            for result in &results {
                println!();
                print_git_metadata(&result.git);
                println!("Link metadata:");
                println!("  content session: {}", result.link.session_id);
                if let Some(memory_session_id) = &result.link.memory_session_id {
                    println!("  memory session:  {memory_session_id}");
                }
                println!("  source:          {}", result.link.source);
                println!(
                    "  linked at:       {}",
                    format_epoch(Some(result.link.linked_at_epoch))
                );
                print_summary(result.link.summary.as_ref());
            }
        }
    }
    Ok(())
}

fn print_commit_lookup(result: &git_trace::CommitLookup) {
    print_git_metadata(&result.git);
    println!("Linked sessions:");
    if result.sessions.is_empty() {
        println!("  none");
        return;
    }
    for session in &result.sessions {
        println!("  content session: {}", session.session_id);
        if let Some(memory_session_id) = &session.memory_session_id {
            println!("  memory session:  {memory_session_id}");
        }
        println!("  source:          {}", session.source);
        println!(
            "  linked at:       {}",
            format_epoch(Some(session.linked_at_epoch))
        );
        print_summary(session.summary.as_ref());
    }
}

fn print_git_metadata(git: &git_trace::GitCommitRecord) {
    println!("Commit {}", git.short_sha);
    println!("Git metadata:");
    println!("  full sha:  {}", git.sha);
    println!("  project:   {}", git.project);
    println!("  repo:      {}", git.repo_path);
    if let Some(branch) = &git.branch {
        println!("  branch:    {branch}");
    }
    if let Some(message) = &git.message {
        println!("  message:   {message}");
    }
    println!("  authored:  {}", format_epoch(git.authored_at_epoch));
    if git.changed_files.is_empty() {
        println!("  files:     none recorded");
    } else {
        println!("  files:");
        for file in &git.changed_files {
            println!("    {file}");
        }
    }
}

fn print_summary(summary: Option<&git_trace::SessionSummaryTrace>) {
    let Some(summary) = summary else {
        println!("  memory summary: none linked");
        return;
    };
    println!("  memory summary:");
    print_optional_summary_field("request", summary.request.as_deref());
    print_optional_summary_field("completed", summary.completed.as_deref());
    print_optional_summary_field("decisions", summary.decisions.as_deref());
    print_optional_summary_field("learned", summary.learned.as_deref());
    print_optional_summary_field("next steps", summary.next_steps.as_deref());
    print_optional_summary_field("preferences", summary.preferences.as_deref());
}

fn print_optional_summary_field(label: &str, value: Option<&str>) {
    if let Some(value) = value {
        println!("    {label}: {value}");
    }
}

fn format_epoch(epoch: Option<i64>) -> String {
    epoch
        .and_then(|value| chrono::DateTime::from_timestamp(value, 0))
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}
