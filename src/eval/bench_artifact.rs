mod report;
mod types;
mod verify;

#[cfg(test)]
mod tests;

pub use report::{
    generate_public_baseline_report, render_public_baseline_markdown, write_public_baseline_report,
    BaselineReportEntry, BenchReportOptions, PublicBaselineReport,
};
pub use types::{
    BenchVerifyFailure, BenchVerifyOptions, BenchVerifyReport, BenchmarkLayer, CodingRunArtifact,
    MemoryCitationEvidence, MemoryDiagnosis, MemoryRetrievalEvidence, MemoryRunArtifact,
    PublicBenchmarkManifest, PublicBenchmarkReport, ReportVerifierMetadata, RunEnvironment,
};
pub use verify::verify_benchmark_artifacts;
