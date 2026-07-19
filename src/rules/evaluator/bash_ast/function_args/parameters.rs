use super::{expand_default_fields, split_fields};

pub(super) fn positional_collection_operator_fields(
    expression: &str,
    zero_argument: Option<&str>,
    arguments: &[String],
    depth: usize,
) -> Option<Vec<String>> {
    let suffix = expression
        .strip_prefix('@')
        .or_else(|| expression.strip_prefix('*'))?;
    let is_set = !arguments.is_empty();
    let is_nonempty = arguments.iter().any(|value| !value.is_empty());
    let argument_fields = || {
        arguments
            .iter()
            .flat_map(|value| split_fields(value))
            .collect::<Vec<_>>()
    };

    if let Some(fallback) = suffix.strip_prefix(":-") {
        return if is_nonempty {
            Some(argument_fields())
        } else {
            expand_default_fields(fallback, zero_argument, arguments, depth + 1)
        };
    }
    if let Some(fallback) = suffix.strip_prefix('-') {
        return if is_set {
            Some(argument_fields())
        } else {
            expand_default_fields(fallback, zero_argument, arguments, depth + 1)
        };
    }
    if let Some(alternative) = suffix.strip_prefix(":+") {
        return if is_nonempty {
            expand_default_fields(alternative, zero_argument, arguments, depth + 1)
        } else {
            Some(Vec::new())
        };
    }
    if let Some(alternative) = suffix.strip_prefix('+') {
        return if is_set {
            expand_default_fields(alternative, zero_argument, arguments, depth + 1)
        } else {
            Some(Vec::new())
        };
    }
    None
}
