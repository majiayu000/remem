use anyhow::{bail, Result};
use toml_edit::{value, Array, DocumentMut, Item, Table};

const DEFAULT_AUTO_PROMOTE_MIN_CONFIDENCE: f64 = 0.7;
const STRICT_AUTO_PROMOTE_MIN_CONFIDENCE: f64 = 0.9;
const DEFAULT_AUTO_PROMOTE_SOURCE_KIND: &str = "explicit_user_statement";
const DEFAULT_AUTO_PROMOTE_REQUIRE_TEXT_SUPPORT: bool = true;
const DEFAULT_AUTO_PROMOTE_STRICT: bool = false;

#[derive(Clone, Debug, PartialEq)]
pub struct UserContextAutoPromoteConfig {
    pub min_confidence: f64,
    pub allowed_source_kinds: Vec<String>,
    pub require_text_support: bool,
    pub strict: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AutoPromotePolicy {
    pub min_confidence: f64,
    pub allowed_source_kinds: Vec<String>,
    pub require_text_support: bool,
}

impl AutoPromotePolicy {
    pub fn strict() -> Self {
        Self {
            min_confidence: STRICT_AUTO_PROMOTE_MIN_CONFIDENCE,
            allowed_source_kinds: default_source_kinds(),
            require_text_support: true,
        }
    }
}

impl UserContextAutoPromoteConfig {
    pub fn effective_policy(&self) -> AutoPromotePolicy {
        if self.strict {
            return AutoPromotePolicy::strict();
        }
        AutoPromotePolicy {
            min_confidence: self.min_confidence,
            allowed_source_kinds: self.allowed_source_kinds.clone(),
            require_text_support: self.require_text_support,
        }
    }
}

pub fn user_context_auto_promote_config() -> Result<UserContextAutoPromoteConfig> {
    let mut doc = super::read_config_doc_or_default()?;
    ensure_defaults(&mut doc)?;
    user_context_auto_promote_config_from_doc(&doc)
}

pub(super) fn ensure_defaults(doc: &mut DocumentMut) -> Result<()> {
    let user_context = super::top_table_mut(doc, "user_context")?;
    let auto_promote = super::child_table_mut(user_context, "auto_promote")?;
    set_f64_if_missing(
        auto_promote,
        "min_confidence",
        DEFAULT_AUTO_PROMOTE_MIN_CONFIDENCE,
    );
    set_string_array_if_missing(
        auto_promote,
        "allowed_source_kinds",
        &[DEFAULT_AUTO_PROMOTE_SOURCE_KIND],
    );
    super::set_bool_if_missing(
        auto_promote,
        "require_text_support",
        DEFAULT_AUTO_PROMOTE_REQUIRE_TEXT_SUPPORT,
    );
    super::set_bool_if_missing(auto_promote, "strict", DEFAULT_AUTO_PROMOTE_STRICT);
    Ok(())
}

fn user_context_auto_promote_config_from_doc(
    doc: &DocumentMut,
) -> Result<UserContextAutoPromoteConfig> {
    let Some(table) = doc
        .get("user_context")
        .and_then(Item::as_table)
        .and_then(|table| table.get("auto_promote"))
        .and_then(Item::as_table)
    else {
        return Ok(default_config());
    };

    let min_confidence = match table.get("min_confidence") {
        Some(item) => {
            parse_auto_promote_confidence(item, "user_context.auto_promote.min_confidence")?
        }
        None => DEFAULT_AUTO_PROMOTE_MIN_CONFIDENCE,
    };
    let allowed_source_kinds = match table.get("allowed_source_kinds") {
        Some(item) => parse_source_kinds(item)?,
        None => default_source_kinds(),
    };
    let require_text_support = match table.get("require_text_support") {
        Some(item) => item.as_bool().ok_or_else(|| {
            anyhow::anyhow!("user_context.auto_promote.require_text_support must be a boolean")
        })?,
        None => DEFAULT_AUTO_PROMOTE_REQUIRE_TEXT_SUPPORT,
    };
    let strict = match table.get("strict") {
        Some(item) => item
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("user_context.auto_promote.strict must be a boolean"))?,
        None => DEFAULT_AUTO_PROMOTE_STRICT,
    };

    Ok(UserContextAutoPromoteConfig {
        min_confidence,
        allowed_source_kinds,
        require_text_support,
        strict,
    })
}

fn default_config() -> UserContextAutoPromoteConfig {
    UserContextAutoPromoteConfig {
        min_confidence: DEFAULT_AUTO_PROMOTE_MIN_CONFIDENCE,
        allowed_source_kinds: default_source_kinds(),
        require_text_support: DEFAULT_AUTO_PROMOTE_REQUIRE_TEXT_SUPPORT,
        strict: DEFAULT_AUTO_PROMOTE_STRICT,
    }
}

fn default_source_kinds() -> Vec<String> {
    vec![DEFAULT_AUTO_PROMOTE_SOURCE_KIND.to_string()]
}

fn parse_auto_promote_confidence(item: &Item, field: &str) -> Result<f64> {
    let value = item
        .as_float()
        .or_else(|| item.as_integer().map(|value| value as f64))
        .ok_or_else(|| anyhow::anyhow!("{field} must be a number"))?;
    if !(0.0..=1.0).contains(&value) {
        bail!("{field} must be between 0.0 and 1.0, got {value}");
    }
    Ok(value)
}

fn parse_source_kinds(item: &Item) -> Result<Vec<String>> {
    let array = item.as_array().ok_or_else(|| {
        anyhow::anyhow!("user_context.auto_promote.allowed_source_kinds must be an array")
    })?;
    let mut values = Vec::new();
    for (index, value) in array.iter().enumerate() {
        let Some(raw) = value.as_str() else {
            bail!(
                "user_context.auto_promote.allowed_source_kinds[{}] must be a string",
                index + 1
            );
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            bail!(
                "user_context.auto_promote.allowed_source_kinds[{}] must not be empty",
                index + 1
            );
        }
        if !values.iter().any(|existing| existing == trimmed) {
            values.push(trimmed.to_string());
        }
    }
    if values.is_empty() {
        bail!("user_context.auto_promote.allowed_source_kinds must not be empty");
    }
    Ok(values)
}

fn set_f64_if_missing(table: &mut Table, key: &str, value_f64: f64) {
    if table.get(key).is_none() {
        table[key] = value(value_f64);
    }
}

fn set_string_array_if_missing(table: &mut Table, key: &str, values: &[&str]) {
    if table.get(key).is_none() {
        let mut array = Array::new();
        for value in values {
            array.push(*value);
        }
        table[key] = value(array);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_user_context_config_path<T>(path: &std::path::Path, f: impl FnOnce() -> T) -> T {
        let _guard = super::super::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let old = std::env::var("REMEM_CONFIG").ok();
        unsafe { std::env::set_var("REMEM_CONFIG", path) };
        let result = f();
        match old {
            Some(value) => unsafe { std::env::set_var("REMEM_CONFIG", value) },
            None => unsafe { std::env::remove_var("REMEM_CONFIG") },
        }
        result
    }

    fn user_context_config_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "remem-{label}-{}-{}.toml",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    #[test]
    fn default_config_exposes_user_context_auto_promote_defaults() {
        let text = super::super::default_config_text();
        assert!(text.contains("[user_context.auto_promote]"), "{text}");
        assert!(text.contains("min_confidence = 0.7"), "{text}");
        assert!(
            text.contains("allowed_source_kinds = [\"explicit_user_statement\"]"),
            "{text}"
        );
        assert!(text.contains("require_text_support = true"), "{text}");
        assert!(text.contains("strict = false"), "{text}");
    }

    #[test]
    fn auto_promote_config_uses_defaults_when_section_is_missing() -> Result<()> {
        let path = user_context_config_path("user-context-auto-promote-missing");
        with_user_context_config_path(&path, || -> Result<()> {
            std::fs::write(&path, "version = 1\n")?;
            let config = user_context_auto_promote_config()?;
            assert_eq!(config, default_config());
            assert_eq!(
                config.effective_policy(),
                AutoPromotePolicy {
                    min_confidence: 0.7,
                    allowed_source_kinds: vec!["explicit_user_statement".to_string()],
                    require_text_support: true,
                }
            );
            Ok(())
        })?;
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn auto_promote_config_reads_valid_values() -> Result<()> {
        let path = user_context_config_path("user-context-auto-promote-valid");
        with_user_context_config_path(&path, || -> Result<()> {
            std::fs::write(
                &path,
                "[user_context.auto_promote]\nmin_confidence = 0.75\nallowed_source_kinds = [\"explicit_user_statement\", \"inferred_from_behavior\", \"explicit_user_statement\"]\nrequire_text_support = false\nstrict = false\n",
            )?;
            let config = user_context_auto_promote_config()?;
            assert_eq!(config.min_confidence, 0.75);
            assert_eq!(
                config.allowed_source_kinds,
                vec![
                    "explicit_user_statement".to_string(),
                    "inferred_from_behavior".to_string()
                ]
            );
            assert!(!config.require_text_support);
            assert!(!config.strict);
            assert_eq!(
                config.effective_policy(),
                AutoPromotePolicy {
                    min_confidence: 0.75,
                    allowed_source_kinds: vec![
                        "explicit_user_statement".to_string(),
                        "inferred_from_behavior".to_string()
                    ],
                    require_text_support: false,
                }
            );
            Ok(())
        })?;
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn strict_auto_promote_config_restores_old_policy() -> Result<()> {
        let path = user_context_config_path("user-context-auto-promote-strict");
        with_user_context_config_path(&path, || -> Result<()> {
            std::fs::write(
                &path,
                "[user_context.auto_promote]\nmin_confidence = 0.5\nallowed_source_kinds = [\"inferred_from_behavior\"]\nrequire_text_support = false\nstrict = true\n",
            )?;
            let config = user_context_auto_promote_config()?;
            assert_eq!(config.effective_policy(), AutoPromotePolicy::strict());
            Ok(())
        })?;
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn auto_promote_config_rejects_invalid_confidence() -> Result<()> {
        for (label, value) in [
            ("negative", "-0.1"),
            ("too-high", "1.1"),
            ("string", "\"0.7\""),
        ] {
            let path = user_context_config_path(&format!("user-context-auto-promote-{label}"));
            with_user_context_config_path(&path, || -> Result<()> {
                std::fs::write(
                    &path,
                    format!("[user_context.auto_promote]\nmin_confidence = {value}\n"),
                )?;
                let err = user_context_auto_promote_config()
                    .expect_err("invalid min_confidence must fail closed");
                assert!(err.to_string().contains("min_confidence"), "{err}");
                Ok(())
            })?;
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    #[test]
    fn auto_promote_config_rejects_invalid_source_kinds() -> Result<()> {
        for (label, value) in [
            ("string", "\"explicit_user_statement\""),
            ("empty-array", "[]"),
            ("empty-string", "[\"\"]"),
            ("non-string", "[1]"),
        ] {
            let path =
                user_context_config_path(&format!("user-context-auto-promote-source-{label}"));
            with_user_context_config_path(&path, || -> Result<()> {
                std::fs::write(
                    &path,
                    format!("[user_context.auto_promote]\nallowed_source_kinds = {value}\n"),
                )?;
                let err = user_context_auto_promote_config()
                    .expect_err("invalid allowed_source_kinds must fail closed");
                assert!(err.to_string().contains("allowed_source_kinds"), "{err}");
                Ok(())
            })?;
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    #[test]
    fn auto_promote_config_rejects_invalid_booleans() -> Result<()> {
        for (label, key) in [("support", "require_text_support"), ("strict", "strict")] {
            let path = user_context_config_path(&format!("user-context-auto-promote-bool-{label}"));
            with_user_context_config_path(&path, || -> Result<()> {
                std::fs::write(
                    &path,
                    format!("[user_context.auto_promote]\n{key} = \"true\"\n"),
                )?;
                let err = user_context_auto_promote_config()
                    .expect_err("invalid boolean must fail closed");
                assert!(err.to_string().contains(key), "{err}");
                Ok(())
            })?;
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
