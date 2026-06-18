use anyhow::Result;

use crate::cli::types::ModelAction;
use crate::runtime_config::{ModelChange, ModelStatus, MODEL_PRESETS};

pub(in crate::cli) async fn run_model(action: ModelAction) -> Result<()> {
    match action {
        ModelAction::Current { host, profile } => {
            if host.is_none() && profile.is_none() {
                render_statuses(&crate::runtime_config::model_statuses()?);
            } else {
                let status =
                    crate::runtime_config::model_status(host.as_deref(), profile.as_deref())?;
                render_statuses(&[status]);
            }
        }
        ModelAction::List => render_presets(),
        ModelAction::Use {
            target,
            host,
            profile,
            reasoning,
            dry_run,
        } => {
            let change = crate::runtime_config::set_model(
                host.as_deref(),
                profile.as_deref(),
                &target,
                reasoning.as_deref(),
                dry_run,
            )?;
            render_change(&change);
        }
        ModelAction::Test {
            host,
            profile,
            live,
        } => {
            let status = crate::runtime_config::model_status(host.as_deref(), profile.as_deref())?;
            render_statuses(std::slice::from_ref(&status));
            if live {
                let response = crate::ai::call_ai(
                    "You are testing a memory AI profile. Reply with exactly: ok",
                    "Return exactly ok.",
                    crate::ai::UsageContext {
                        project: None,
                        session_id: None,
                        operation: "model_test",
                        host: profile.is_none().then_some(host.as_deref()).flatten(),
                        profile: profile.as_deref(),
                    },
                )
                .await?;
                println!("live_test -> {}", response.trim());
            } else {
                println!("config_check -> ok");
                println!("live_call -> skipped (pass --live to call the configured model)");
            }
        }
        ModelAction::Rollback => {
            let (path, backup_path) = crate::runtime_config::rollback_model_config()?;
            println!("restored -> {}", path.display());
            println!("from     -> {}", backup_path.display());
        }
    }
    Ok(())
}

fn render_statuses(statuses: &[ModelStatus]) {
    println!(
        "{:<13} {:<12} {:<10} {:<16} Reasoning",
        "Host", "Profile", "Executor", "Model"
    );
    for status in statuses {
        println!(
            "{:<13} {:<12} {:<10} {:<16} {}",
            status.host.as_deref().unwrap_or("-"),
            status.profile_name,
            status.executor.as_str(),
            status.model,
            status.reasoning_effort.as_deref().unwrap_or("-")
        );
    }
    if let Some(status) = statuses.first() {
        println!("Config: {}", status.config_path.display());
    }
}

fn render_presets() {
    println!("{:<10} {:<16} {:<9} Notes", "Preset", "Model", "Reasoning");
    for preset in MODEL_PRESETS {
        println!(
            "{:<10} {:<16} {:<9} {}",
            preset.name,
            preset.model,
            preset.reasoning_effort.unwrap_or("-"),
            preset.description
        );
    }
    println!();
    println!("Examples:");
    println!("  remem model use cheap");
    println!("  remem model use balanced --dry-run");
    println!("  remem model use gpt-5.2 --reasoning medium");
    println!("  remem model use haiku --host claude-code");
    println!("  remem model use auto");
    println!();
    println!("Claude Code profiles use explicit model names; presets are Codex-focused.");
}

fn render_change(change: &ModelChange) {
    println!(
        "Profile {}{}",
        change.profile_name,
        change
            .host
            .as_ref()
            .map(|host| format!(" (host {host})"))
            .unwrap_or_default()
    );
    println!("Executor: {}", change.executor.as_str());
    println!("Config: {}", change.config_path.display());
    if change.dry_run {
        println!("Mode: dry-run (no files written)");
    } else if let Some(path) = &change.backup_path {
        println!("Backup: {}", path.display());
    }
    print_diff_line("model", Some(&change.old_model), Some(&change.new_model));
    print_diff_line(
        "reasoning_effort",
        change.old_reasoning_effort.as_deref(),
        change.new_reasoning_effort.as_deref(),
    );
}

fn print_diff_line(key: &str, old: Option<&str>, new: Option<&str>) {
    if old == new {
        println!("  {key} = {}", quote_or_dash(new));
        return;
    }
    println!("- {key} = {}", quote_or_dash(old));
    println!("+ {key} = {}", quote_or_dash(new));
}

fn quote_or_dash(value: Option<&str>) -> String {
    value
        .map(|value| format!("\"{value}\""))
        .unwrap_or_else(|| "-".to_string())
}
