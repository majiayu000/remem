pub(crate) fn common_string<'a>(mut values: impl Iterator<Item = &'a str>) -> Option<String> {
    let first = values.next()?.to_string();
    values.all(|value| value == first).then_some(first)
}

pub(crate) fn common_optional_string<'a>(
    mut values: impl Iterator<Item = Option<&'a str>>,
) -> Option<String> {
    let first = values.next()??.to_string();
    values
        .all(|value| value.is_some_and(|value| value == first))
        .then_some(first)
}

pub(crate) fn common_optional_i64(mut values: impl Iterator<Item = Option<i64>>) -> Option<i64> {
    let first = values.next()??;
    values
        .all(|value| value.is_some_and(|value| value == first))
        .then_some(first)
}
