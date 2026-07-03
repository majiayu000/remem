mod display;
pub(in crate::eval) mod run;
#[cfg(test)]
mod tests;
mod types;
pub(in crate::eval) mod validation;

pub use run::{
    evaluate_dataset, evaluate_dataset_with_fixture_corpus, load_dataset, run_dataset,
    run_dataset_path,
};
pub use types::{
    CategoryEvaluation, EvidenceRef, GoldenDataset, GoldenEvalReport, GoldenHopPath, GoldenMemory,
    GoldenQuery, MetricAverages, QueryEvaluation, QueryMetrics, QueryStatus,
};
