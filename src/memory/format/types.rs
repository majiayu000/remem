#[derive(Debug, Clone, PartialEq)]
pub struct ParsedObservation {
    pub obs_type: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub facts: Vec<String>,
    pub narrative: Option<String>,
    pub concepts: Vec<String>,
    pub files_read: Vec<String>,
    pub files_modified: Vec<String>,
    /// Model-provided confidence clamped to [0.0, 1.0]; None when missing or unparseable.
    pub confidence: Option<f64>,
}
