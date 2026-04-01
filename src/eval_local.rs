mod dedup;
mod display;
mod project_leak;
mod run;
mod self_retrieval;
#[cfg(test)]
mod tests;
mod title_quality;
mod types;

pub use run::run_eval;
pub use types::{
    DedupReport, EvalReport, ProjectLeakReport, SelfRetrievalReport, TitleQualityReport,
};
