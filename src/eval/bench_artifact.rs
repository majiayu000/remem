mod authority;
mod report;
mod types;
mod verify;

#[cfg(test)]
mod tests;

pub use report::{
    generate_public_baseline_report, render_public_baseline_markdown, write_public_baseline_report,
    BaselineReportEntry, BenchReportOptions, CodingPairedStatistic, PublicBaselineReport,
};
pub(crate) use types::VerifiedArtifact;
pub use types::{
    AuthorityStatus, AuthorityVerdict, BenchVerifyFailure, BenchVerifyOptions, BenchVerifyReport,
    BenchmarkLayer, CodingRunArtifact, Gh931AuthorityVerdict, Gh931ReportBinding,
    MemoryCitationEvidence, MemoryDiagnosis, MemoryRetrievalEvidence, MemoryRunArtifact,
    PublicBenchmarkManifest, PublicBenchmarkReport, ReleaseAuthorityVerdict,
    ReportVerifierMetadata, RunEnvironment, SecurityAuthorityVerdict,
    SecurityReportAuthorityVerdict,
};
pub use verify::verify_benchmark_artifacts;
