mod detail;
mod report;
mod summary;
#[cfg(test)]
mod tests;
mod types;

pub use report::generate_timeline_report;
pub(crate) use report::{generate_timeline_report_data, TimelineReportData};
