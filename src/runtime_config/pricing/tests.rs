use super::*;

fn with_config_path<T>(path: &std::path::Path, f: impl FnOnce() -> T) -> T {
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

fn temp_config_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "remem-{label}-{}-{}.toml",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}

#[test]
fn default_config_contains_empty_pricing_table() {
    let text = super::super::default_config_text();
    assert!(text.contains("[pricing]"), "{text}");
    assert!(
        !text.contains("input_per_mtok"),
        "init must not pin compiled rates: {text}"
    );
}

#[test]
fn missing_section_is_no_override() -> Result<()> {
    let path = temp_config_path("pricing-missing");
    with_config_path(&path, || -> Result<()> {
        std::fs::write(&path, "version = 1\n")?;
        assert_eq!(global_pricing_override()?, None);
        Ok(())
    })?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn global_section_overrides_when_both_rates_are_set() -> Result<()> {
    let path = temp_config_path("pricing-global");
    with_config_path(&path, || -> Result<()> {
        std::fs::write(
            &path,
            "[pricing]\ninput_per_mtok = 1.25\noutput_per_mtok = 6.5\n",
        )?;
        let rates = global_pricing_override()?.expect("global override");
        assert_eq!(rates.input_per_mtok, 1.25);
        assert_eq!(rates.output_per_mtok, 6.5);
        assert_eq!(rates.reasoning_per_mtok, 6.5);
        assert_eq!(rates.cache_read_per_mtok, 1.25);
        Ok(())
    })?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn global_section_accepts_integer_rates() -> Result<()> {
    let path = temp_config_path("pricing-int");
    with_config_path(&path, || -> Result<()> {
        std::fs::write(
            &path,
            "[pricing]\ninput_per_mtok = 2\noutput_per_mtok = 8\n",
        )?;
        let rates = global_pricing_override()?.expect("global override");
        assert_eq!(rates.input_per_mtok, 2.0);
        assert_eq!(rates.output_per_mtok, 8.0);
        Ok(())
    })?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn one_sided_or_invalid_global_section_fails_closed() -> Result<()> {
    for (label, body) in [
        ("input-only", "[pricing]\ninput_per_mtok = 1.25\n"),
        (
            "string",
            "[pricing]\ninput_per_mtok = \"1.25\"\noutput_per_mtok = 6.5\n",
        ),
        (
            "negative",
            "[pricing]\ninput_per_mtok = -1\noutput_per_mtok = 6.5\n",
        ),
        (
            "unknown-key",
            "[pricing]\ninput_per_mtok = 1.0\noutput_per_mtok = 2.0\nfoo = 1\n",
        ),
        (
            "optional-without-pair",
            "[pricing]\nreasoning_per_mtok = 3.0\n",
        ),
    ] {
        let path = temp_config_path(&format!("pricing-{label}"));
        with_config_path(&path, || -> Result<()> {
            std::fs::write(&path, body)?;
            let err = global_pricing_override().expect_err("invalid pricing must fail");
            assert!(err.to_string().contains("pricing"), "{err}");
            Ok(())
        })?;
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn family_table_overlays_selected_fields() -> Result<()> {
    let path = temp_config_path("pricing-family");
    with_config_path(&path, || -> Result<()> {
        std::fs::write(&path, "[pricing.haiku]\ninput_per_mtok = 2.5\n")?;
        let overlay =
            family_pricing_overlay("HAIKU", PricingRates::from_parts(1.0, 5.0, 1.25, 0.10))?;
        assert_eq!(overlay.input_per_mtok, 2.5);
        assert_eq!(overlay.output_per_mtok, 5.0);
        Ok(())
    })?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn unknown_family_table_fails_closed() -> Result<()> {
    let path = temp_config_path("pricing-unknown-family");
    with_config_path(&path, || -> Result<()> {
        std::fs::write(&path, "[pricing.gpt52]\ninput_per_mtok = 1.0\n")?;
        let err = validate_pricing_config().expect_err("unknown family");
        assert!(err.to_string().contains("gpt52"), "{err}");
        Ok(())
    })?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn pricing_can_be_set_through_config_cli() -> Result<()> {
    let path = temp_config_path("pricing-cli");
    with_config_path(&path, || -> Result<()> {
        super::super::init_config()?;
        super::super::set_config_value("pricing.input_per_mtok", "1.25")?;
        super::super::set_config_value("pricing.output_per_mtok", "6.5")?;
        let rates = global_pricing_override()?.expect("global override");
        assert_eq!(rates.input_per_mtok, 1.25);
        assert_eq!(rates.output_per_mtok, 6.5);
        let text = std::fs::read_to_string(&path)?;
        assert!(text.contains("input_per_mtok = 1.25"), "{text}");
        Ok(())
    })?;
    std::fs::remove_file(path)?;
    Ok(())
}
