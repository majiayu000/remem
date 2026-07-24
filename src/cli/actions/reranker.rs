use anyhow::Result;

use crate::cli::reranker_types::RerankerAction;

pub(in crate::cli) fn run_reranker(action: RerankerAction) -> Result<()> {
    match action {
        RerankerAction::Download { model, json } => {
            let report = crate::retrieval::rerank::download_reranker_model(model.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "Downloaded reranker model {} ({}) into {}.",
                    report.model_id, report.upstream_model, report.install_dir
                );
                println!(
                    "Verified {} model files (manifest sha256 {}).",
                    report.files_verified, report.manifest_sha256
                );
            }
            Ok(())
        }
        RerankerAction::Status { json } => {
            let report = crate::retrieval::rerank::reranker_status()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "Rerank: enabled={} state={} preset={} top_n={} top_k={}",
                    report.enabled, report.state, report.preset, report.top_n, report.top_k
                );
                println!("Model: {} ({})", report.model_id, report.upstream_model);
                println!("Install dir: {}", report.install_dir);
                if let Some(manifest_sha256) = &report.manifest_sha256 {
                    println!("Manifest sha256: {manifest_sha256}");
                }
                if let Some(reason) = &report.disabled_reason {
                    println!("Disabled reason: {reason}");
                }
                if let Some(detail) = &report.detail {
                    println!("Detail: {detail}");
                }
            }
            Ok(())
        }
    }
}
