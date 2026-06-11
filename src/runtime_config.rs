use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use toml_edit::{value, DocumentMut, Item, Table};

mod model;
pub use model::{
    model_status, model_statuses, rollback_model_config, set_model, ModelChange, ModelPreset,
    ModelStatus, MODEL_PRESETS,
};

pub const CLAUDE_HOST: &str = "claude-code";
pub const CODEX_HOST: &str = "codex-cli";
pub const DEFAULT_CODEX_MODEL: &str = "gpt-5.2";
pub const MEMORY_AI_PROFILE_FIELD: &str = "remem_ai_profile";

const DEFAULT_CLAUDE_MODEL: &str = "haiku";
const ANTHROPIC_DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryAiExecutor {
    Http,
    ClaudeCli,
    CodexCli,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryAiSelection<'a> {
    pub host: Option<&'a str>,
    pub profile: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedMemoryAiProfile {
    pub profile_name: String,
    pub executor: MemoryAiExecutor,
    pub model: Option<String>,
    pub cli_path: Option<String>,
    pub base_url: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostRuntimeConfig {
    pub host: String,
    pub memory_profile: String,
    pub context_gate: Option<String>,
    pub context_color: bool,
    pub capture_adapter: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigDefaultMode {
    PreserveUserValues,
    MigrateLegacyDefaults,
}

pub fn config_path() -> PathBuf {
    std::env::var("REMEM_CONFIG")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::db::data_dir().join("config.toml"))
}

pub fn default_config_text() -> String {
    let mut doc = DocumentMut::new();
    ensure_config_defaults_with_mode(
        &mut doc,
        &[CLAUDE_HOST, CODEX_HOST],
        ConfigDefaultMode::MigrateLegacyDefaults,
    )
    .expect("default runtime config should be valid");
    doc.to_string()
}

pub fn show_config_text() -> Result<String> {
    let mut doc = read_config_doc_or_default()?;
    ensure_config_defaults(&mut doc, &[CLAUDE_HOST, CODEX_HOST])?;
    Ok(doc.to_string())
}

pub fn init_config() -> Result<PathBuf> {
    let path = config_path();
    let mut doc = read_config_doc_or_default()?;
    ensure_config_defaults_with_mode(
        &mut doc,
        &[CLAUDE_HOST, CODEX_HOST],
        ConfigDefaultMode::MigrateLegacyDefaults,
    )?;
    write_config_doc(&path, &doc)?;
    Ok(path)
}

pub fn ensure_config_for_hosts(hosts: &[&str]) -> Result<PathBuf> {
    let path = config_path();
    let mut doc = read_config_doc_or_default()?;
    ensure_config_defaults_with_mode(&mut doc, hosts, ConfigDefaultMode::MigrateLegacyDefaults)?;
    write_config_doc(&path, &doc)?;
    Ok(path)
}

pub fn set_config_value(key: &str, raw_value: &str) -> Result<PathBuf> {
    let path = config_path();
    let mut doc = read_config_doc_or_default()?;
    ensure_config_defaults_with_mode(
        &mut doc,
        &[CLAUDE_HOST, CODEX_HOST],
        ConfigDefaultMode::MigrateLegacyDefaults,
    )?;

    let segments = key
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        bail!("config key must not be empty");
    }

    let mut current = doc.as_table_mut();
    for segment in &segments[..segments.len().saturating_sub(1)] {
        current = child_table_mut(current, segment)?;
    }
    let leaf = segments[segments.len() - 1];
    current[leaf] = cli_value(raw_value);
    write_config_doc(&path, &doc)?;
    Ok(path)
}

pub fn normalize_host(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "claude" | "claude-code" | "claudecode" => CLAUDE_HOST.to_string(),
        "codex" | "codex-cli" | "codexcli" => CODEX_HOST.to_string(),
        "unknown" => "unknown".to_string(),
        _ => raw.trim().to_string(),
    }
}

pub(crate) fn profile_from_payload_text(input: &str) -> Option<String> {
    let payload: serde_json::Value = serde_json::from_str(input).ok()?;
    payload
        .as_object()?
        .get(MEMORY_AI_PROFILE_FIELD)?
        .as_str()
        .map(str::trim)
        .filter(|profile| !profile.is_empty())
        .map(str::to_string)
}

pub fn default_host() -> Result<String> {
    let mut doc = read_config_doc_or_default()?;
    ensure_config_defaults(&mut doc, &[CLAUDE_HOST, CODEX_HOST])?;
    Ok(configured_default_host(&doc))
}

pub fn resolve_host_runtime_config(host: Option<&str>) -> Result<HostRuntimeConfig> {
    let mut doc = read_config_doc_or_default()?;
    let selected_host = host
        .map(normalize_host)
        .filter(|host| !host.trim().is_empty());
    match selected_host.as_deref() {
        Some(host) => ensure_config_defaults(&mut doc, &[CLAUDE_HOST, CODEX_HOST, host])?,
        None => ensure_config_defaults(&mut doc, &[CLAUDE_HOST, CODEX_HOST])?,
    }
    let host = selected_host.unwrap_or_else(|| configured_default_host(&doc));
    host_runtime_config_from_doc(&doc, &host)
}

pub fn resolve_memory_ai_profile(
    selection: MemoryAiSelection<'_>,
) -> Result<ResolvedMemoryAiProfile> {
    if selection.host.is_some() && selection.profile.is_some() {
        bail!("--host and --profile are mutually exclusive");
    }

    let mut doc = read_config_doc_or_default()?;
    let selected_host = selection
        .host
        .map(normalize_host)
        .filter(|host| !host.trim().is_empty());
    match selected_host.as_deref() {
        Some(host) => ensure_config_defaults(&mut doc, &[CLAUDE_HOST, CODEX_HOST, host])?,
        None => ensure_config_defaults(&mut doc, &[CLAUDE_HOST, CODEX_HOST])?,
    }
    let profile_name = match selection.profile {
        Some(profile) if !profile.trim().is_empty() => profile.trim().to_string(),
        _ => {
            let host = selected_host.unwrap_or_else(|| configured_default_host(&doc));
            host_runtime_config_from_doc(&doc, &host)?.memory_profile
        }
    };
    profile_from_doc(&doc, &profile_name)
}

fn read_config_doc_or_default() -> Result<DocumentMut> {
    let path = config_path();
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    content
        .parse::<DocumentMut>()
        .with_context(|| format!("parse {} as TOML", path.display()))
}

fn write_config_doc(path: &PathBuf, doc: &DocumentMut) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create config dir {}", parent.display()))?;
    }
    std::fs::write(path, doc.to_string()).with_context(|| format!("write {}", path.display()))
}

fn ensure_config_defaults(doc: &mut DocumentMut, hosts: &[&str]) -> Result<()> {
    ensure_config_defaults_with_mode(doc, hosts, ConfigDefaultMode::PreserveUserValues)
}

fn ensure_config_defaults_with_mode(
    doc: &mut DocumentMut,
    hosts: &[&str],
    mode: ConfigDefaultMode,
) -> Result<()> {
    if doc.get("version").is_none() {
        doc["version"] = value(1);
    }

    let memory_ai = top_table_mut(doc, "memory_ai")?;
    set_str_if_missing(memory_ai, "default_host", CODEX_HOST);
    let default_host = memory_ai
        .get("default_host")
        .and_then(Item::as_str)
        .map(normalize_host)
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| CODEX_HOST.to_string());

    {
        let profiles = child_table_mut(memory_ai, "profiles")?;
        ensure_codex_profile(profiles)?;
        ensure_claude_profile(profiles)?;
        ensure_http_profile(profiles)?;
    }

    {
        let hosts_table = child_table_mut(memory_ai, "hosts")?;
        ensure_host_config(hosts_table, &default_host, mode)?;
        for host in hosts {
            let host = normalize_host(host);
            if !host.is_empty() && host != default_host {
                ensure_host_config(hosts_table, &host, mode)?;
            }
        }
    }

    Ok(())
}

fn ensure_codex_profile(profiles: &mut Table) -> Result<()> {
    let profile = child_table_mut(profiles, "codex")?;
    set_str_if_missing(profile, "executor", "codex-cli");
    set_str_if_missing(profile, "model", DEFAULT_CODEX_MODEL);
    set_str_if_missing(profile, "path", "codex");
    Ok(())
}

fn ensure_claude_profile(profiles: &mut Table) -> Result<()> {
    let profile = child_table_mut(profiles, "claude")?;
    set_str_if_missing(profile, "executor", "claude-cli");
    set_str_if_missing(profile, "model", DEFAULT_CLAUDE_MODEL);
    set_str_if_missing(profile, "path", "claude");
    Ok(())
}

fn ensure_http_profile(profiles: &mut Table) -> Result<()> {
    let profile = child_table_mut(profiles, "anthropic_http")?;
    set_str_if_missing(profile, "executor", "http");
    set_str_if_missing(profile, "model", DEFAULT_CLAUDE_MODEL);
    set_str_if_missing(profile, "base_url", ANTHROPIC_DEFAULT_BASE_URL);
    Ok(())
}

fn ensure_host_config(hosts: &mut Table, host: &str, mode: ConfigDefaultMode) -> Result<()> {
    let table = child_table_mut(hosts, host)?;
    match host {
        CODEX_HOST => {
            set_str_if_missing(table, "memory_profile", "codex");
            set_str_if_missing(table, "context_gate", "strict");
            set_bool_if_missing(table, "context_color", true);
            set_str_if_missing(table, "capture_adapter", CODEX_HOST);
        }
        CLAUDE_HOST => {
            set_str_if_missing(table, "memory_profile", "claude");
            if mode == ConfigDefaultMode::MigrateLegacyDefaults {
                set_str_if_missing_or_legacy_value(table, "context_gate", "auto", "off");
            } else {
                set_str_if_missing(table, "context_gate", "auto");
            }
            set_bool_if_missing(table, "context_color", true);
            set_str_if_missing(table, "capture_adapter", CLAUDE_HOST);
        }
        "unknown" => {
            set_str_if_missing(table, "memory_profile", "codex");
            set_str_if_missing(table, "context_gate", "off");
            set_bool_if_missing(table, "context_color", false);
            set_str_if_missing(table, "capture_adapter", "unknown");
        }
        _ => {
            set_str_if_missing(table, "memory_profile", "codex");
            set_str_if_missing(table, "context_gate", "off");
            set_bool_if_missing(table, "context_color", false);
            set_str_if_missing(table, "capture_adapter", host);
        }
    }
    Ok(())
}

fn configured_default_host(doc: &DocumentMut) -> String {
    doc.get("memory_ai")
        .and_then(Item::as_table)
        .and_then(|table| table.get("default_host"))
        .and_then(Item::as_str)
        .map(normalize_host)
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| CODEX_HOST.to_string())
}

fn host_runtime_config_from_doc(doc: &DocumentMut, host: &str) -> Result<HostRuntimeConfig> {
    let Some(hosts) = doc
        .get("memory_ai")
        .and_then(Item::as_table)
        .and_then(|table| table.get("hosts"))
        .and_then(Item::as_table)
    else {
        bail!("missing [memory_ai.hosts] in {}", config_path().display());
    };
    let Some(table) = hosts.get(host).and_then(Item::as_table) else {
        bail!(
            "missing [memory_ai.hosts.\"{}\"] in {}",
            host,
            config_path().display()
        );
    };
    let memory_profile = required_str(table, "memory_profile")?.to_string();
    let context_gate = optional_str(table, "context_gate");
    let context_color = table
        .get("context_color")
        .and_then(Item::as_bool)
        .unwrap_or(false);
    let capture_adapter =
        optional_str(table, "capture_adapter").unwrap_or_else(|| host.to_string());

    Ok(HostRuntimeConfig {
        host: host.to_string(),
        memory_profile,
        context_gate,
        context_color,
        capture_adapter,
    })
}

fn profile_from_doc(doc: &DocumentMut, profile_name: &str) -> Result<ResolvedMemoryAiProfile> {
    let Some(profiles) = doc
        .get("memory_ai")
        .and_then(Item::as_table)
        .and_then(|table| table.get("profiles"))
        .and_then(Item::as_table)
    else {
        bail!(
            "missing [memory_ai.profiles] in {}",
            config_path().display()
        );
    };
    let Some(table) = profiles.get(profile_name).and_then(Item::as_table) else {
        bail!(
            "missing [memory_ai.profiles.{}] in {}",
            profile_name,
            config_path().display()
        );
    };
    let executor = parse_executor(required_str(table, "executor")?)?;
    let model = optional_str(table, "model").and_then(|model| {
        if model.eq_ignore_ascii_case("auto") {
            None
        } else {
            Some(model)
        }
    });
    Ok(ResolvedMemoryAiProfile {
        profile_name: profile_name.to_string(),
        executor,
        model,
        cli_path: optional_str(table, "path"),
        base_url: optional_str(table, "base_url"),
        reasoning_effort: optional_str(table, "reasoning_effort"),
    })
}

fn parse_executor(raw: &str) -> Result<MemoryAiExecutor> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "http" | "anthropic" | "anthropic-http" => Ok(MemoryAiExecutor::Http),
        "claude" | "cli" | "claude-cli" => Ok(MemoryAiExecutor::ClaudeCli),
        "codex" | "codex-cli" => Ok(MemoryAiExecutor::CodexCli),
        other => bail!("unknown memory_ai executor: {other}"),
    }
}

fn top_table_mut<'a>(doc: &'a mut DocumentMut, key: &str) -> Result<&'a mut Table> {
    doc.entry(key)
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .with_context(|| format!("{key} exists but is not a table"))
}

fn child_table_mut<'a>(table: &'a mut Table, key: &str) -> Result<&'a mut Table> {
    table
        .entry(key)
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .with_context(|| format!("{key} exists but is not a table"))
}

fn set_str_if_missing(table: &mut Table, key: &str, value_str: &str) {
    if table.get(key).is_none() {
        table[key] = value(value_str);
    }
}

fn set_str_if_missing_or_legacy_value(
    table: &mut Table,
    key: &str,
    value_str: &str,
    legacy_value: &str,
) {
    let should_set = table
        .get(key)
        .and_then(Item::as_str)
        .map(|current| current.trim().eq_ignore_ascii_case(legacy_value))
        .unwrap_or(true);
    if should_set {
        table[key] = value(value_str);
    }
}

fn set_bool_if_missing(table: &mut Table, key: &str, value_bool: bool) {
    if table.get(key).is_none() {
        table[key] = value(value_bool);
    }
}

fn required_str<'a>(table: &'a Table, key: &str) -> Result<&'a str> {
    table
        .get(key)
        .and_then(Item::as_str)
        .with_context(|| format!("missing or invalid string key '{key}'"))
}

fn optional_str(table: &Table, key: &str) -> Option<String> {
    table
        .get(key)
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn cli_value(raw: &str) -> Item {
    let trimmed = raw.trim();
    match trimmed.to_ascii_lowercase().as_str() {
        "true" => value(true),
        "false" => value(false),
        _ => match trimmed.parse::<i64>() {
            Ok(number) => value(number),
            Err(_) => value(trim_outer_quotes(trimmed)),
        },
    }
}

fn trim_outer_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_config_path<T>(path: &std::path::Path, f: impl FnOnce() -> T) -> T {
        let _guard = TEST_ENV_LOCK.lock().expect("env lock should acquire");
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
            "remem-{label}-{}-{}.toml",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    #[test]
    fn default_config_contains_codex_quality_profile() {
        let text = default_config_text();
        assert!(text.contains("default_host = \"codex-cli\""), "{text}");
        assert!(text.contains("model = \"gpt-5.2\""), "{text}");
        assert!(!text.contains("gpt-5.4-mini"), "{text}");
        assert!(!text.contains("reasoning_effort = \"low\""), "{text}");
    }

    #[test]
    fn host_selection_resolves_to_profile() {
        let path = temp_config_path("runtime-resolve");
        with_config_path(&path, || {
            init_config().unwrap();
            let profile = resolve_memory_ai_profile(MemoryAiSelection {
                host: Some("codex"),
                profile: None,
            })
            .unwrap();

            assert_eq!(profile.profile_name, "codex");
            assert_eq!(profile.executor, MemoryAiExecutor::CodexCli);
            assert_eq!(profile.model.as_deref(), Some(DEFAULT_CODEX_MODEL));
        });
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn explicit_profile_bypasses_host_mapping() {
        let path = temp_config_path("runtime-profile");
        with_config_path(&path, || {
            init_config().unwrap();
            let profile = resolve_memory_ai_profile(MemoryAiSelection {
                host: None,
                profile: Some("claude"),
            })
            .unwrap();

            assert_eq!(profile.executor, MemoryAiExecutor::ClaudeCli);
            assert_eq!(profile.model.as_deref(), Some("haiku"));
        });
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn host_and_profile_are_mutually_exclusive() {
        let err = resolve_memory_ai_profile(MemoryAiSelection {
            host: Some(CODEX_HOST),
            profile: Some("codex"),
        })
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
    }

    #[test]
    fn set_config_value_updates_nested_key() {
        let path = temp_config_path("runtime-set");
        with_config_path(&path, || {
            init_config().unwrap();
            set_config_value("memory_ai.profiles.codex.model", "custom-mini").unwrap();
            let profile = resolve_memory_ai_profile(MemoryAiSelection {
                host: Some(CODEX_HOST),
                profile: None,
            })
            .unwrap();
            assert_eq!(profile.model.as_deref(), Some("custom-mini"));
        });
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn context_options_resolve_from_host_config() {
        let path = temp_config_path("runtime-context");
        with_config_path(&path, || {
            init_config().unwrap();
            let host = resolve_host_runtime_config(Some("codex")).unwrap();

            assert_eq!(host.host, CODEX_HOST);
            assert_eq!(host.context_gate.as_deref(), Some("strict"));
            assert!(host.context_color);
            assert_eq!(host.capture_adapter, CODEX_HOST);
        });
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn claude_host_defaults_to_context_gate_auto() -> Result<()> {
        let path = temp_config_path("runtime-claude-context");
        with_config_path(&path, || -> Result<()> {
            init_config()?;
            let host = resolve_host_runtime_config(Some("claude-code"))?;

            assert_eq!(host.host, CLAUDE_HOST);
            assert_eq!(host.context_gate.as_deref(), Some("auto"));
            assert!(host.context_color);
            assert_eq!(host.capture_adapter, CLAUDE_HOST);
            Ok(())
        })?;
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn claude_host_migrates_legacy_context_gate_off_to_auto() -> Result<()> {
        let path = temp_config_path("runtime-claude-context-migrate");
        with_config_path(&path, || -> Result<()> {
            std::fs::write(
                &path,
                "[memory_ai.hosts.claude-code]\nmemory_profile = \"claude\"\ncontext_gate = \"off\"\n",
            )?;
            init_config()?;
            let host = resolve_host_runtime_config(Some("claude-code"))?;

            assert_eq!(host.host, CLAUDE_HOST);
            assert_eq!(host.context_gate.as_deref(), Some("auto"));
            Ok(())
        })?;
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn claude_host_resolve_preserves_explicit_context_gate_off() -> Result<()> {
        let path = temp_config_path("runtime-claude-context-explicit-off");
        with_config_path(&path, || -> Result<()> {
            std::fs::write(
                &path,
                "[memory_ai.hosts.claude-code]\nmemory_profile = \"claude\"\ncontext_gate = \"off\"\n",
            )?;
            let host = resolve_host_runtime_config(Some("claude-code"))?;

            assert_eq!(host.host, CLAUDE_HOST);
            assert_eq!(host.context_gate.as_deref(), Some("off"));
            Ok(())
        })?;
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn partial_install_still_materializes_configured_default_host() -> Result<()> {
        let path = temp_config_path("runtime-partial-default-host");
        with_config_path(&path, || -> Result<()> {
            ensure_config_for_hosts(&[CLAUDE_HOST])?;
            let text = std::fs::read_to_string(&path)?;

            assert!(text.contains("[memory_ai.hosts.claude-code]"), "{text}");
            assert!(text.contains("[memory_ai.hosts.codex-cli]"), "{text}");

            let host = resolve_host_runtime_config(None)?;
            assert_eq!(host.host, CODEX_HOST);
            assert_eq!(host.memory_profile, "codex");
            Ok(())
        })?;
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn explicit_unknown_host_materializes_fallback_config() -> Result<()> {
        let path = temp_config_path("runtime-unknown-host");
        with_config_path(&path, || -> Result<()> {
            init_config()?;
            let host = resolve_host_runtime_config(Some("unknown"))?;
            let profile = resolve_memory_ai_profile(MemoryAiSelection {
                host: Some("unknown"),
                profile: None,
            })?;

            assert_eq!(host.host, "unknown");
            assert_eq!(host.memory_profile, "codex");
            assert_eq!(host.context_gate.as_deref(), Some("off"));
            assert_eq!(profile.profile_name, "codex");
            Ok(())
        })?;
        std::fs::remove_file(path)?;
        Ok(())
    }
}
