pub(super) fn positional_collection_slice(
    expression: &str,
    zero_argument: Option<&str>,
    arguments: &[String],
) -> Option<Vec<String>> {
    let slice = expression.strip_prefix("@:")?;
    let (offset, length) = nonnegative_slice(slice)?;
    let mut values = if offset == 0 {
        std::iter::once(zero_argument?.to_string())
            .chain(arguments.iter().cloned())
            .collect::<Vec<_>>()
    } else {
        arguments.get(offset - 1..).unwrap_or_default().to_vec()
    };
    if let Some(length) = length {
        values.truncate(length);
    }
    Some(values)
}

pub(super) fn positional_substring(value: &str, slice: &str) -> Option<String> {
    let (offset, length) = nonnegative_slice(slice)?;
    let characters = value.chars().skip(offset);
    Some(match length {
        Some(length) => characters.take(length).collect(),
        None => characters.collect(),
    })
}

fn nonnegative_slice(source: &str) -> Option<(usize, Option<usize>)> {
    let (offset, length) = match source.split_once(':') {
        Some((offset, length)) => (offset, Some(length.parse::<usize>().ok()?)),
        None => (source, None),
    };
    Some((offset.parse::<usize>().ok()?, length))
}
