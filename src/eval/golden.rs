mod display;
mod run;
#[cfg(test)]
mod tests;
mod types;

pub use run::{evaluate_dataset, load_dataset, run_dataset_path};
pub use types::{
    EvidenceRef, GoldenDataset, GoldenEvalReport, GoldenQuery, MetricAverages, QueryEvaluation,
    QueryMetrics, QueryStatus,
};
