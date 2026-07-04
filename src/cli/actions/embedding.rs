use anyhow::Result;
use serde::Serialize;

use crate::cli::embedding_types::EmbeddingAction;

pub(in crate::cli) fn run_embedding(action: EmbeddingAction) -> Result<()> {
    match action {
        EmbeddingAction::Download { model, json } => {
            let report =
                crate::retrieval::embedding::download_local_embedding_model(model.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "Downloaded local embedding model {} ({}, {} dims) into {}.",
                    report.model_id, report.upstream_model, report.dimensions, report.install_dir
                );
                println!("Verified {} model files.", report.files_verified);
            }
            Ok(())
        }
        EmbeddingAction::Status { json } => run_embedding_status(json),
        EmbeddingAction::Backfill {
            batch,
            limit,
            prune,
            json,
        } => {
            super::query::run_embedding_backfill(limit, batch, prune, json)?;
            Ok(())
        }
    }
}

fn run_embedding_status(json: bool) -> Result<()> {
    let provider = crate::retrieval::embedding::embedding_provider_status()?;
    let inventory = crate::retrieval::embedding::local_embedding_inventory()?;
    let report = EmbeddingStatusCliReport {
        configured_provider: provider.configured_provider,
        fallback_provider: provider.fallback_provider,
        active_provider: provider.active_provider,
        active_model_id: provider.active_model_id,
        active_dimensions: provider.active_dimensions,
        degraded: provider.degraded,
        disabled: provider.disabled,
        unavailable_reason: provider.unavailable_reason,
        degradation_reason: provider.degradation_reason,
        model_root: inventory.model_root,
        configured_preset: inventory.configured_preset,
        models: inventory.models,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Embedding provider: {} -> {}",
            report.configured_provider, report.active_provider
        );
        if let Some(fallback) = &report.fallback_provider {
            println!("Fallback: {}", fallback);
        }
        println!(
            "Active model: {}",
            report.active_model_id.as_deref().unwrap_or("none")
        );
        println!(
            "State: degraded={} disabled={}",
            report.degraded, report.disabled
        );
        if let Some(reason) = &report.unavailable_reason {
            println!("Unavailable: {}", reason);
        } else if let Some(reason) = &report.degradation_reason {
            println!("Degraded: {}", reason);
        }
        println!("Model root: {}", report.model_root);
        println!("Configured preset: {}", report.configured_preset);
        for model in &report.models {
            let state = if model.installed {
                "installed"
            } else {
                "missing"
            };
            println!(
                "  {}: {} ({} dims) {}",
                model.preset, model.model_id, model.dimensions, state
            );
            if let Some(reason) = &model.unavailable_reason {
                println!("    {}", reason);
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct EmbeddingStatusCliReport {
    configured_provider: String,
    fallback_provider: Option<String>,
    active_provider: String,
    active_model_id: Option<String>,
    active_dimensions: Option<usize>,
    degraded: bool,
    disabled: bool,
    unavailable_reason: Option<String>,
    degradation_reason: Option<String>,
    model_root: String,
    configured_preset: String,
    models: Vec<crate::retrieval::embedding::LocalEmbeddingModelInventory>,
}
