use anyhow::{bail, Result};
use toml_edit::{DocumentMut, Item};

const DEFAULT_RULE_COMPILATION_ENABLED: bool = false;
const DEFAULT_RULE_COMPILE_MIN_REINFORCEMENT: i64 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuleCompilationConfig {
    pub enabled: bool,
    pub min_reinforcement: i64,
}

pub fn rule_compilation_config() -> Result<RuleCompilationConfig> {
    let mut doc = super::read_config_doc_or_default()?;
    ensure_defaults(&mut doc)?;
    rule_compilation_config_from_doc(&doc)
}

pub(super) fn ensure_defaults(doc: &mut DocumentMut) -> Result<()> {
    let rules = super::top_table_mut(doc, "rule_compilation")?;
    super::set_bool_if_missing(rules, "enabled", DEFAULT_RULE_COMPILATION_ENABLED);
    super::set_i64_if_missing(
        rules,
        "rule_compile_min_reinforcement",
        DEFAULT_RULE_COMPILE_MIN_REINFORCEMENT,
    );
    Ok(())
}

fn rule_compilation_config_from_doc(doc: &DocumentMut) -> Result<RuleCompilationConfig> {
    let Some(table) = doc.get("rule_compilation").and_then(Item::as_table) else {
        return Ok(RuleCompilationConfig {
            enabled: DEFAULT_RULE_COMPILATION_ENABLED,
            min_reinforcement: DEFAULT_RULE_COMPILE_MIN_REINFORCEMENT,
        });
    };
    let enabled = table
        .get("enabled")
        .and_then(Item::as_bool)
        .unwrap_or(DEFAULT_RULE_COMPILATION_ENABLED);
    let min_reinforcement = table
        .get("rule_compile_min_reinforcement")
        .and_then(Item::as_integer)
        .unwrap_or(DEFAULT_RULE_COMPILE_MIN_REINFORCEMENT);
    if min_reinforcement < 1 {
        bail!(
            "rule_compilation.rule_compile_min_reinforcement must be >= 1, got {min_reinforcement}"
        );
    }
    Ok(RuleCompilationConfig {
        enabled,
        min_reinforcement,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_rules_config_path<T>(path: &std::path::Path, f: impl FnOnce() -> T) -> T {
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

    fn rules_config_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "remem-{label}-{}-{}.toml",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    #[test]
    fn default_config_disables_rule_compilation() {
        let text = super::super::default_config_text();
        assert!(text.contains("[rule_compilation]"), "{text}");
        assert!(text.contains("enabled = false"), "{text}");
        assert!(
            text.contains("rule_compile_min_reinforcement = 3"),
            "{text}"
        );
    }

    #[test]
    fn rule_compilation_config_reads_enabled_and_threshold() -> Result<()> {
        let path = rules_config_path("rule-compilation-config");
        with_rules_config_path(&path, || -> Result<()> {
            super::super::init_config()?;
            super::super::set_config_value("rule_compilation.enabled", "true")?;
            super::super::set_config_value("rule_compilation.rule_compile_min_reinforcement", "5")?;

            let config = rule_compilation_config()?;
            assert!(config.enabled);
            assert_eq!(config.min_reinforcement, 5);
            Ok(())
        })?;
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn rule_compilation_config_rejects_zero_threshold() -> Result<()> {
        let path = rules_config_path("rule-compilation-zero");
        with_rules_config_path(&path, || -> Result<()> {
            std::fs::write(
                &path,
                "[rule_compilation]\nrule_compile_min_reinforcement = 0\n",
            )?;
            let err = rule_compilation_config().expect_err("zero threshold must fail closed");
            assert!(
                err.to_string()
                    .contains("rule_compile_min_reinforcement must be >= 1"),
                "{err}"
            );
            Ok(())
        })?;
        std::fs::remove_file(path)?;
        Ok(())
    }
}
