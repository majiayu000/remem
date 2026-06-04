use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use toml_edit::{value, Item, Table};

use super::{
    config_path, ensure_config_defaults, host_runtime_config_from_doc, normalize_host,
    profile_from_doc, read_config_doc_or_default, write_config_doc, MemoryAiExecutor, CLAUDE_HOST,
    CODEX_HOST, DEFAULT_CODEX_MODEL,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelPreset {
    pub name: &'static str,
    pub model: &'static str,
    pub reasoning_effort: Option<&'static str>,
    pub description: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelStatus {
    pub host: Option<String>,
    pub profile_name: String,
    pub executor: MemoryAiExecutor,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub config_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChange {
    pub host: Option<String>,
    pub profile_name: String,
    pub executor: MemoryAiExecutor,
    pub old_model: String,
    pub new_model: String,
    pub old_reasoning_effort: Option<String>,
    pub new_reasoning_effort: Option<String>,
    pub config_path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub dry_run: bool,
}

pub const MODEL_PRESETS: &[ModelPreset] = &[
    ModelPreset {
        name: "cheap",
        model: DEFAULT_CODEX_MODEL,
        reasoning_effort: Some("low"),
        description: "lowest default Codex cost; current install default",
    },
    ModelPreset {
        name: "balanced",
        model: DEFAULT_CODEX_MODEL,
        reasoning_effort: Some("medium"),
        description: "same Codex model with more reasoning for extraction quality",
    },
    ModelPreset {
        name: "quality",
        model: "gpt-5.2",
        reasoning_effort: Some("medium"),
        description: "higher-quality Codex profile; higher cost",
    },
    ModelPreset {
        name: "auto",
        model: "auto",
        reasoning_effort: None,
        description: "omit --model and let Codex choose its default",
    },
];

impl MemoryAiExecutor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::ClaudeCli => "claude-cli",
            Self::CodexCli => "codex-cli",
        }
    }
}

pub fn model_statuses() -> Result<Vec<ModelStatus>> {
    [CODEX_HOST, CLAUDE_HOST]
        .iter()
        .map(|host| model_status(Some(host), None))
        .collect()
}

pub fn model_status(host: Option<&str>, profile: Option<&str>) -> Result<ModelStatus> {
    let mut doc = read_config_doc_or_default()?;
    ensure_config_defaults(&mut doc, &[CLAUDE_HOST, CODEX_HOST])?;
    let selection = select_profile_from_doc(&doc, host, profile)?;
    let resolved = profile_from_doc(&doc, &selection.profile_name)?;
    Ok(ModelStatus {
        host: selection.host,
        profile_name: resolved.profile_name,
        executor: resolved.executor,
        model: resolved.model.unwrap_or_else(|| "auto".to_string()),
        reasoning_effort: resolved.reasoning_effort,
        config_path: config_path(),
    })
}

pub fn set_model(
    host: Option<&str>,
    profile: Option<&str>,
    target: &str,
    reasoning_effort: Option<&str>,
    dry_run: bool,
) -> Result<ModelChange> {
    let path = config_path();
    let mut doc = read_config_doc_or_default()?;
    ensure_config_defaults(&mut doc, &[CLAUDE_HOST, CODEX_HOST])?;
    let selection = select_profile_from_doc(&doc, host, profile)?;
    let before = profile_from_doc(&doc, &selection.profile_name)?;
    let target = resolve_model_target(target, reasoning_effort, before.executor)?;
    let old_model = before.model.clone().unwrap_or_else(|| "auto".to_string());
    let old_reasoning_effort = before.reasoning_effort.clone();
    let new_reasoning_effort = if target.update_reasoning {
        target.reasoning_effort.clone()
    } else {
        old_reasoning_effort.clone()
    };
    let change = ModelChange {
        host: selection.host.clone(),
        profile_name: selection.profile_name.clone(),
        executor: before.executor,
        old_model,
        new_model: target.model.clone(),
        old_reasoning_effort,
        new_reasoning_effort: new_reasoning_effort.clone(),
        config_path: path.clone(),
        backup_path: (!dry_run).then(|| backup_path_for_config(&path)),
        dry_run,
    };
    if dry_run {
        return Ok(change);
    }

    let backup_path = backup_path_for_config(&path);
    write_config_doc(&backup_path, &doc)?;
    let profile_table = profile_table_mut(&mut doc, &selection.profile_name)?;
    profile_table["model"] = value(target.model);
    match new_reasoning_effort {
        Some(reasoning) => profile_table["reasoning_effort"] = value(reasoning),
        None => {
            profile_table.remove("reasoning_effort");
        }
    }
    write_config_doc(&path, &doc)?;
    Ok(change)
}

pub fn rollback_model_config() -> Result<(PathBuf, PathBuf)> {
    let path = config_path();
    let backup_path = backup_path_for_config(&path);
    if !backup_path.exists() {
        bail!(
            "no model config backup found at {}; run `remem model use ...` first",
            backup_path.display()
        );
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create config dir {}", parent.display()))?;
    }
    std::fs::copy(&backup_path, &path)
        .with_context(|| format!("restore {} from {}", path.display(), backup_path.display()))?;
    Ok((path, backup_path))
}

struct ProfileSelection {
    host: Option<String>,
    profile_name: String,
}

struct ModelTarget {
    model: String,
    reasoning_effort: Option<String>,
    update_reasoning: bool,
}

fn select_profile_from_doc(
    doc: &toml_edit::DocumentMut,
    host: Option<&str>,
    profile: Option<&str>,
) -> Result<ProfileSelection> {
    if host.is_some() && profile.is_some() {
        bail!("--host and --profile are mutually exclusive");
    }
    if let Some(profile) = profile.map(str::trim).filter(|profile| !profile.is_empty()) {
        return Ok(ProfileSelection {
            host: None,
            profile_name: profile.to_string(),
        });
    }
    let host = host
        .map(normalize_host)
        .filter(|host| !host.trim().is_empty())
        .unwrap_or_else(|| super::configured_default_host(doc));
    let profile_name = host_runtime_config_from_doc(doc, &host)?.memory_profile;
    Ok(ProfileSelection {
        host: Some(host),
        profile_name,
    })
}

fn resolve_model_target(
    target: &str,
    reasoning_effort: Option<&str>,
    executor: MemoryAiExecutor,
) -> Result<ModelTarget> {
    let target = target.trim();
    if target.is_empty() {
        bail!("model or preset must not be empty");
    }
    let lower = target.to_ascii_lowercase();
    if let Some(preset) = MODEL_PRESETS.iter().find(|preset| preset.name == lower) {
        if executor != MemoryAiExecutor::CodexCli {
            bail!(
                "model preset '{}' is for codex-cli profiles; pass an explicit model for {}",
                preset.name,
                executor.as_str()
            );
        }
        if reasoning_effort.is_some() && preset.name == "auto" {
            bail!("--reasoning cannot be used with `auto`");
        }
        return Ok(ModelTarget {
            model: preset.model.to_string(),
            reasoning_effort: reasoning_effort
                .map(normalize_reasoning_effort)
                .transpose()?
                .or_else(|| preset.reasoning_effort.map(str::to_string)),
            update_reasoning: true,
        });
    }

    if lower == "auto" && executor != MemoryAiExecutor::CodexCli {
        bail!("model `auto` is only supported for codex-cli profiles");
    }
    if lower == "auto" && reasoning_effort.is_some() {
        bail!("--reasoning cannot be used with `auto`");
    }
    Ok(ModelTarget {
        model: canonical_model_name(target),
        reasoning_effort: reasoning_effort
            .map(normalize_reasoning_effort)
            .transpose()?,
        update_reasoning: reasoning_effort.is_some() || lower == "auto",
    })
}

fn canonical_model_name(model: &str) -> String {
    match model.trim().to_ascii_lowercase().as_str() {
        "5.4-mini" | "gpt5-4.mini" | "gpt-5-4-mini" => DEFAULT_CODEX_MODEL.to_string(),
        "5.2" | "gpt5.2" | "gpt-5-2" => "gpt-5.2".to_string(),
        other => other.to_string(),
    }
}

fn normalize_reasoning_effort(reasoning_effort: &str) -> Result<String> {
    match reasoning_effort.trim().to_ascii_lowercase().as_str() {
        "low" | "medium" | "high" => Ok(reasoning_effort.trim().to_ascii_lowercase()),
        other => bail!("unknown reasoning effort '{other}'; expected low, medium, or high"),
    }
}

fn profile_table_mut<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    profile_name: &str,
) -> Result<&'a mut Table> {
    doc.get_mut("memory_ai")
        .and_then(Item::as_table_mut)
        .and_then(|table| table.get_mut("profiles"))
        .and_then(Item::as_table_mut)
        .and_then(|profiles| profiles.get_mut(profile_name))
        .and_then(Item::as_table_mut)
        .with_context(|| format!("missing [memory_ai.profiles.{profile_name}]"))
}

fn backup_path_for_config(path: &Path) -> PathBuf {
    let mut backup = path.to_path_buf();
    backup.set_extension("toml.bak");
    backup
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_config_path<T>(path: &Path, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().expect("env lock should acquire");
        let old = std::env::var("REMEM_CONFIG").ok();
        unsafe { std::env::set_var("REMEM_CONFIG", path) };
        let result = f();
        match old {
            Some(value) => unsafe { std::env::set_var("REMEM_CONFIG", value) },
            None => unsafe { std::env::remove_var("REMEM_CONFIG") },
        }
        result
    }

    fn temp_config_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "remem-model-{label}-{}-{}.toml",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    #[test]
    fn model_use_preset_updates_codex_profile_and_backup() {
        let path = temp_config_path("preset");
        with_config_path(&path, || {
            super::super::init_config().unwrap();
            let change = set_model(Some(CODEX_HOST), None, "balanced", None, false).unwrap();
            assert_eq!(change.old_model, DEFAULT_CODEX_MODEL);
            assert_eq!(change.new_model, DEFAULT_CODEX_MODEL);
            assert_eq!(change.new_reasoning_effort.as_deref(), Some("medium"));
            assert!(change.backup_path.as_ref().unwrap().exists());

            let status = model_status(Some(CODEX_HOST), None).unwrap();
            assert_eq!(status.reasoning_effort.as_deref(), Some("medium"));
        });
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(backup_path_for_config(&path));
    }

    #[test]
    fn model_use_dry_run_does_not_write() {
        let path = temp_config_path("dry-run");
        with_config_path(&path, || {
            super::super::init_config().unwrap();
            let change = set_model(Some(CODEX_HOST), None, "quality", None, true).unwrap();
            assert!(change.dry_run);
            assert_eq!(change.new_model, "gpt-5.2");

            let status = model_status(Some(CODEX_HOST), None).unwrap();
            assert_eq!(status.model, DEFAULT_CODEX_MODEL);
        });
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn model_rollback_restores_backup() {
        let path = temp_config_path("rollback");
        with_config_path(&path, || {
            super::super::init_config().unwrap();
            set_model(Some(CODEX_HOST), None, "quality", None, false).unwrap();
            rollback_model_config().unwrap();

            let status = model_status(Some(CODEX_HOST), None).unwrap();
            assert_eq!(status.model, DEFAULT_CODEX_MODEL);
            assert_eq!(status.reasoning_effort.as_deref(), Some("low"));
        });
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(backup_path_for_config(&path));
    }
}
