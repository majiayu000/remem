use anyhow::{bail, Result};
use toml_edit::{DocumentMut, Table};

const RATE_KEYS: &[&str] = &[
    "input_per_mtok",
    "output_per_mtok",
    "reasoning_per_mtok",
    "cache_creation_per_mtok",
    "cache_read_per_mtok",
];

const FAMILY_TABLES: &[&str] = &[
    "opus",
    "sonnet",
    "haiku",
    "gpt55",
    "gpt54",
    "gpt54_mini",
    "gpt54_nano",
    "gpt5_codex",
    "gpt5",
    "gpt4",
    "codex_mini",
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PricingRates {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub reasoning_per_mtok: f64,
    pub cache_creation_per_mtok: f64,
    pub cache_read_per_mtok: f64,
}

impl PricingRates {
    pub(crate) fn from_parts(
        input: f64,
        output: f64,
        cache_creation: f64,
        cache_read: f64,
    ) -> Self {
        Self {
            input_per_mtok: input,
            output_per_mtok: output,
            reasoning_per_mtok: output,
            cache_creation_per_mtok: cache_creation,
            cache_read_per_mtok: cache_read,
        }
    }
}

pub(super) fn ensure_defaults(doc: &mut DocumentMut) -> Result<()> {
    let _ = super::top_table_mut(doc, "pricing")?;
    Ok(())
}

pub(crate) fn validate_pricing_config() -> Result<()> {
    let doc = super::read_config_doc_or_default()?;
    let _ = global_pricing_from_doc(&doc)?;
    for family in FAMILY_TABLES {
        let _ =
            family_pricing_from_doc(&doc, family, PricingRates::from_parts(0.0, 0.0, 0.0, 0.0))?;
    }
    Ok(())
}

pub(crate) fn global_pricing_override() -> Result<Option<PricingRates>> {
    let doc = super::read_config_doc_or_default()?;
    global_pricing_from_doc(&doc)
}

pub(crate) fn family_pricing_overlay(
    prefix: &str,
    base: PricingRates,
) -> Result<(PricingRates, bool)> {
    let doc = super::read_config_doc_or_default()?;
    family_pricing_from_doc(&doc, &prefix.to_ascii_lowercase(), base)
}

fn global_pricing_from_doc(doc: &DocumentMut) -> Result<Option<PricingRates>> {
    let Some(pricing) = doc.get("pricing") else {
        return Ok(None);
    };
    let Some(table) = pricing.as_table() else {
        bail!("pricing must be a table");
    };
    reject_unknown_keys(table, RATE_KEYS, FAMILY_TABLES, "pricing")?;
    let input = optional_rate(table, "pricing.input_per_mtok")?;
    let output = optional_rate(table, "pricing.output_per_mtok")?;
    let reasoning = optional_rate(table, "pricing.reasoning_per_mtok")?;
    let cache_creation = optional_rate(table, "pricing.cache_creation_per_mtok")?;
    let cache_read = optional_rate(table, "pricing.cache_read_per_mtok")?;
    match (input, output) {
        (None, None) => {
            if reasoning.is_some() || cache_creation.is_some() || cache_read.is_some() {
                bail!(
                    "pricing.input_per_mtok and pricing.output_per_mtok must both be set \
                     when other [pricing] rates are present"
                );
            }
            Ok(None)
        }
        (Some(input), Some(output)) => Ok(Some(PricingRates {
            input_per_mtok: input,
            output_per_mtok: output,
            reasoning_per_mtok: reasoning.unwrap_or(output),
            cache_creation_per_mtok: cache_creation.unwrap_or(input),
            cache_read_per_mtok: cache_read.unwrap_or(input),
        })),
        _ => bail!("pricing.input_per_mtok and pricing.output_per_mtok must both be set"),
    }
}

fn family_pricing_from_doc(
    doc: &DocumentMut,
    family: &str,
    mut base: PricingRates,
) -> Result<(PricingRates, bool)> {
    let Some(pricing_item) = doc.get("pricing") else {
        return Ok((base, false));
    };
    let Some(pricing) = pricing_item.as_table() else {
        bail!("pricing must be a table");
    };
    let Some(table) = pricing.get(family) else {
        return Ok((base, false));
    };
    let Some(table) = table.as_table() else {
        bail!("pricing.{family} must be a table");
    };
    reject_unknown_keys(table, RATE_KEYS, &[], &format!("pricing.{family}"))?;
    if let Some(value) = optional_rate(table, &format!("pricing.{family}.input_per_mtok"))? {
        base.input_per_mtok = value;
    }
    if let Some(value) = optional_rate(table, &format!("pricing.{family}.output_per_mtok"))? {
        base.output_per_mtok = value;
    }
    if let Some(value) = optional_rate(table, &format!("pricing.{family}.reasoning_per_mtok"))? {
        base.reasoning_per_mtok = value;
    }
    if let Some(value) = optional_rate(table, &format!("pricing.{family}.cache_creation_per_mtok"))?
    {
        base.cache_creation_per_mtok = value;
    }
    if let Some(value) = optional_rate(table, &format!("pricing.{family}.cache_read_per_mtok"))? {
        base.cache_read_per_mtok = value;
    }
    Ok((base, !table.is_empty()))
}

fn reject_unknown_keys(
    table: &Table,
    allowed_keys: &[&str],
    allowed_tables: &[&str],
    path: &str,
) -> Result<()> {
    for (key, item) in table.iter() {
        if allowed_keys.contains(&key) {
            continue;
        }
        if allowed_tables.contains(&key) && item.as_table().is_some() {
            continue;
        }
        bail!("{path} has unknown key '{key}'");
    }
    Ok(())
}

fn optional_rate(table: &Table, path: &str) -> Result<Option<f64>> {
    let Some(item) = table.get(path.rsplit_once('.').map(|(_, key)| key).unwrap_or(path)) else {
        return Ok(None);
    };
    let value = item
        .as_float()
        .or_else(|| item.as_integer().map(|value| value as f64))
        .ok_or_else(|| anyhow::anyhow!("{path} must be a number"))?;
    if !value.is_finite() {
        bail!("{path} must be a finite number, got {value}");
    }
    if value < 0.0 {
        bail!("{path} must be >= 0, got {value}");
    }
    Ok(Some(value))
}

#[cfg(test)]
mod tests;
