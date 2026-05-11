#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedObservation {
    pub obs_type: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub facts: Vec<String>,
    pub narrative: Option<String>,
    pub concepts: Vec<String>,
    pub files_read: Vec<String>,
    pub files_modified: Vec<String>,
}
