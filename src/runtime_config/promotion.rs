use anyhow::{bail, Result};
use toml_edit::{DocumentMut, Item};

const DEFAULT_SUMMARY_GATE_MODE: &str = "enforce";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SummaryGateMode {
    Off,
    Shadow,
    Enforce,
}

pub fn summary_gate_mode() -> Result<SummaryGateMode> {
    let mut doc = super::read_config_doc_or_default()?;
    ensure_defaults(&mut doc)?;
    let mode = doc
        .get("promotion")
        .and_then(Item::as_table)
        .and_then(|table| super::optional_str(table, "summary_gate_mode"))
        .unwrap_or_else(|| DEFAULT_SUMMARY_GATE_MODE.to_string());
    parse_summary_gate_mode(&mode)
}

pub(super) fn ensure_defaults(doc: &mut DocumentMut) -> Result<()> {
    let promotion = super::top_table_mut(doc, "promotion")?;
    super::set_str_if_missing(promotion, "summary_gate_mode", DEFAULT_SUMMARY_GATE_MODE);
    Ok(())
}

fn parse_summary_gate_mode(raw: &str) -> Result<SummaryGateMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "off" => Ok(SummaryGateMode::Off),
        "shadow" => Ok(SummaryGateMode::Shadow),
        "enforce" => Ok(SummaryGateMode::Enforce),
        other => {
            bail!("unknown promotion.summary_gate_mode: {other}; expected off, shadow, or enforce")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_promotion_config_path<T>(path: &std::path::Path, f: impl FnOnce() -> T) -> T {
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

    fn promotion_config_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "remem-{label}-{}-{}.toml",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    #[test]
    fn default_config_enables_summary_gate_enforce_mode() {
        let text = super::super::default_config_text();
        assert!(text.contains("summary_gate_mode = \"enforce\""), "{text}");
    }

    #[test]
    fn summary_gate_mode_reads_config_value() -> Result<()> {
        let path = promotion_config_path("summary-gate-mode");
        with_promotion_config_path(&path, || -> Result<()> {
            super::super::init_config()?;
            super::super::set_config_value("promotion.summary_gate_mode", "shadow")?;
            assert_eq!(summary_gate_mode()?, SummaryGateMode::Shadow);

            super::super::set_config_value("promotion.summary_gate_mode", "off")?;
            assert_eq!(summary_gate_mode()?, SummaryGateMode::Off);
            Ok(())
        })?;
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn summary_gate_mode_rejects_unknown_value() -> Result<()> {
        let path = promotion_config_path("summary-gate-mode-invalid");
        with_promotion_config_path(&path, || -> Result<()> {
            std::fs::write(&path, "[promotion]\nsummary_gate_mode = \"maybe\"\n")?;
            let err = summary_gate_mode().expect_err("invalid mode must fail closed");
            assert!(
                err.to_string()
                    .contains("unknown promotion.summary_gate_mode"),
                "{err}"
            );
            Ok(())
        })?;
        std::fs::remove_file(path)?;
        Ok(())
    }
}
