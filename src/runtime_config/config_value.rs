use toml_edit::{value, Item};

pub(crate) fn cli_value(raw: &str) -> Item {
    let trimmed = raw.trim();
    match trimmed.to_ascii_lowercase().as_str() {
        "true" => value(true),
        "false" => value(false),
        _ => match trimmed.parse::<i64>() {
            Ok(number) => value(number),
            Err(_) => match trimmed.parse::<f64>() {
                Ok(number) if number.is_finite() => value(number),
                _ => value(trim_outer_quotes(trimmed)),
            },
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

    #[test]
    fn cli_value_parses_float_scalars() {
        assert_eq!(cli_value("0.75").as_float(), Some(0.75));
    }

    #[test]
    fn cli_value_keeps_quoted_float_scalars_as_strings() {
        assert_eq!(cli_value("\"0.75\"").as_str(), Some("0.75"));
    }
}
