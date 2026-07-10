use anyhow::Result;

use crate::{db, summarize};

pub(super) async fn run_rule_compilation_sweep() -> Result<usize> {
    tokio::task::spawn_blocking(crate::rules::run_compile_rules_sweep).await?
}

pub(super) async fn process_job(job: &db::Job) -> Result<()> {
    match job.job_type {
        db::JobType::Observation => {
            crate::log::warn(
                "worker",
                &format!(
                    "skipping legacy observation job id={}; captures are processed via extraction_tasks",
                    job.id
                ),
            );
            Ok(())
        }
        db::JobType::Summary => {
            anyhow::bail!(
                "legacy Summary jobs are retired; SessionRollup owns session summary output"
            )
        }
        db::JobType::Compress => {
            let profile = job_profile(&job.payload_json);
            summarize::process_compress_job(&job.host, &job.project, profile.as_deref()).await?;
            Ok(())
        }
        db::JobType::Dream => {
            let profile = job_profile(&job.payload_json);
            if let Some(profile) = profile.as_deref() {
                crate::dream::process_dream_job_with_profile(&job.project, Some(profile)).await?;
            } else {
                crate::dream::process_dream_job_with_host(&job.project, &job.host).await?;
            }
            Ok(())
        }
        db::JobType::CompileRules => {
            let project = job.project.clone();
            tokio::task::spawn_blocking(move || crate::rules::run_compile_rules_job(&project))
                .await??;
            Ok(())
        }
    }
}

fn job_profile(payload_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(payload_json)
        .ok()
        .and_then(|value| {
            value
                .get("remem_ai_profile")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|profile| !profile.is_empty())
                .map(str::to_string)
        })
}
