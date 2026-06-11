use serde::Serialize;

/// One probe in `remem doctor`. `name` is `&'static str` because every probe
/// is statically defined in this crate.
pub(crate) struct Check {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    pub duration_ms: u64,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Status {
    Ok,
    Warn,
    Fail,
}

impl Status {
    /// Short human label. Kept ASCII so the string survives in CI logs that
    /// strip color or non-ASCII bytes.
    pub(crate) fn icon(&self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "WARN",
            Status::Fail => "FAIL",
        }
    }

    /// Stable lowercase tag emitted in `--json` output. Renaming this is a
    /// breaking change for any script that parses the JSON.
    pub(crate) fn as_json_tag(&self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
        }
    }
}

impl Check {
    pub(crate) fn new(name: &'static str, status: Status, detail: impl Into<String>) -> Self {
        Self {
            name,
            status,
            detail: detail.into(),
            duration_ms: 0,
        }
    }

    pub(crate) fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        self
    }

    pub(crate) fn icon(&self) -> &'static str {
        self.status.icon()
    }
}

/// Aggregate result of all probes. Lets the caller decide what to do
/// (e.g. translate to a process exit code) without reaching into individual
/// `Check` values.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DoctorOutcome {
    pub fails: usize,
    pub warns: usize,
}

impl DoctorOutcome {
    /// Map probe results to a process exit code. CI scripts rely on this:
    ///   0  — every check OK
    ///   1  — at least one warning, no failures
    ///   2  — at least one failure
    ///
    /// Splitting warn from fail (rather than collapsing both to 1) lets a
    /// caller block on hard failures while only logging warnings.
    pub(crate) fn exit_code(&self) -> i32 {
        if self.fails > 0 {
            2
        } else if self.warns > 0 {
            1
        } else {
            0
        }
    }
}

/// Current JSON contract version. Bump when removing or renaming a field, or
/// when adding mandatory observability fields to every check/root report.
pub(crate) const REPORT_SCHEMA_VERSION: u32 = 2;

/// JSON-stable shape for `remem doctor --json`. Field names and the
/// `status` tag are part of the CLI's machine-readable contract; do not
/// rename or reorder without bumping `REPORT_SCHEMA_VERSION`.
#[derive(Serialize)]
pub(crate) struct CheckJson<'a> {
    pub name: &'a str,
    pub status: &'a str,
    pub detail: &'a str,
    pub duration_ms: u64,
}

#[derive(Serialize)]
pub(crate) struct ReportJson<'a> {
    pub schema_version: u32,
    pub version: &'a str,
    pub binary_schema_version: i64,
    pub status: &'a str,
    pub fails: usize,
    pub warns: usize,
    pub elapsed_ms: u64,
    pub checks: Vec<CheckJson<'a>>,
}
