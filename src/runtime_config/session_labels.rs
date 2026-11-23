use anyhow::Result;
use toml_edit::{DocumentMut, Item};

pub(crate) const SESSION_LABEL_TIMEZONE_ENV: &str = "REMEM_SESSION_LABEL_TIMEZONE";
pub(crate) const DEFAULT_SESSION_LABEL_TIMEZONE: &str = "local";
/// Always stored as a string by `remem config set`: compact offsets such as
/// `+0800` or `+08` would otherwise become integers the reader ignores.
pub(super) const SESSION_LABEL_TIMEZONE_KEY: &str = "session_labels.timezone";

/// Raw `session_labels.timezone` setting: env override first, then config,
/// then `local`. Parsing happens in `memory::session_label`.
pub(crate) fn session_label_timezone_setting() -> Result<String> {
    let doc = super::read_config_doc_or_default()?;
    let env_value = std::env::var(SESSION_LABEL_TIMEZONE_ENV).ok();
    Ok(setting_from(env_value.as_deref(), &doc))
}

pub(super) fn ensure_defaults(doc: &mut DocumentMut) -> Result<()> {
    let table = super::top_table_mut(doc, "session_labels")?;
    super::set_str_if_missing(table, "timezone", DEFAULT_SESSION_LABEL_TIMEZONE);
    Ok(())
}

fn setting_from(env_value: Option<&str>, doc: &DocumentMut) -> String {
    env_value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            doc.get("session_labels")
                .and_then(Item::as_table)
                .and_then(|table| super::optional_str(table, "timezone"))
        })
        .unwrap_or_else(|| DEFAULT_SESSION_LABEL_TIMEZONE.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> DocumentMut {
        text.parse().expect("test TOML should parse")
    }

    #[test]
    fn default_config_writes_local_session_label_timezone() {
        let text = super::super::default_config_text();
        assert!(
            text.contains("[session_labels]\ntimezone = \"local\""),
            "{text}"
        );
    }

    #[test]
    fn missing_setting_defaults_to_local() {
        assert_eq!(setting_from(None, &doc("")), "local");
        assert_eq!(setting_from(Some("  "), &doc("")), "local");
    }

    #[test]
    fn config_value_is_used_without_env() {
        let doc = doc("[session_labels]\ntimezone = \"+08:00\"\n");
        assert_eq!(setting_from(None, &doc), "+08:00");
    }

    #[test]
    fn env_overrides_config_value() {
        let doc = doc("[session_labels]\ntimezone = \"+08:00\"\n");
        assert_eq!(setting_from(Some(" UTC "), &doc), "UTC");
    }

    #[test]
    fn setting_reads_config_file_and_env_override() -> Result<()> {
        let _guard = super::super::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let path = std::env::temp_dir().join(format!(
            "remem-session-label-tz-{}-{}.toml",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&path, "[session_labels]\ntimezone = \"-05:00\"\n")?;
        let old_config = std::env::var_os("REMEM_CONFIG");
        let old_tz = std::env::var_os(SESSION_LABEL_TIMEZONE_ENV);
        unsafe {
            std::env::set_var("REMEM_CONFIG", &path);
            std::env::remove_var(SESSION_LABEL_TIMEZONE_ENV);
        }
        let from_config = session_label_timezone_setting();
        unsafe { std::env::set_var(SESSION_LABEL_TIMEZONE_ENV, "+09:00") };
        let from_env = session_label_timezone_setting();
        unsafe {
            match old_config {
                Some(value) => std::env::set_var("REMEM_CONFIG", value),
                None => std::env::remove_var("REMEM_CONFIG"),
            }
            match old_tz {
                Some(value) => std::env::set_var(SESSION_LABEL_TIMEZONE_ENV, value),
                None => std::env::remove_var(SESSION_LABEL_TIMEZONE_ENV),
            }
        }
        let _ = std::fs::remove_file(&path);
        assert_eq!(from_config?, "-05:00");
        assert_eq!(from_env?, "+09:00");
        Ok(())
    }

    #[test]
    fn config_set_keeps_compact_offsets_readable() -> Result<()> {
        let _guard = super::super::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let path = std::env::temp_dir().join(format!(
            "remem-session-label-tz-set-{}-{}.toml",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let old_config = std::env::var_os("REMEM_CONFIG");
        let old_tz = std::env::var_os(SESSION_LABEL_TIMEZONE_ENV);
        unsafe {
            std::env::set_var("REMEM_CONFIG", &path);
            std::env::remove_var(SESSION_LABEL_TIMEZONE_ENV);
        }
        let read_back = ["+0800", "+08", "-0530", "\"+09\""]
            .into_iter()
            .map(|raw| {
                super::super::set_config_value(" session_labels . timezone ", raw)?;
                session_label_timezone_setting()
            })
            .collect::<Result<Vec<_>>>();
        unsafe {
            match old_config {
                Some(value) => std::env::set_var("REMEM_CONFIG", value),
                None => std::env::remove_var("REMEM_CONFIG"),
            }
            match old_tz {
                Some(value) => std::env::set_var(SESSION_LABEL_TIMEZONE_ENV, value),
                None => std::env::remove_var(SESSION_LABEL_TIMEZONE_ENV),
            }
        }
        let _ = std::fs::remove_file(&path);
        assert_eq!(read_back?, ["+0800", "+08", "-0530", "+09"]);
        Ok(())
    }
}
