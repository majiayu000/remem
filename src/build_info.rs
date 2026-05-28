pub(crate) fn package_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub(crate) fn binary_schema_version() -> i64 {
    crate::migrate::latest_schema_version()
}

pub(crate) fn version_label() -> String {
    format!(
        "{} (schema v{})",
        package_version(),
        binary_schema_version()
    )
}
