mod display;
mod run;
#[cfg(test)]
mod tests;
mod types;

pub use run::{
    evaluate_dataset, evaluate_dataset_with_fixture_corpus, load_dataset, run_dataset,
    run_dataset_path,
};
pub use types::{
    EvidenceRef, GoldenDataset, GoldenEvalReport, GoldenMemory, GoldenQuery, MetricAverages,
    QueryEvaluation, QueryMetrics, QueryStatus,
};
