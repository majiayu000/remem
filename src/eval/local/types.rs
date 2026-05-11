pub struct EvalReport {
    pub total_memories: i64,
    pub dedup: DedupReport,
    pub project_leak: ProjectLeakReport,
    pub title_quality: TitleQualityReport,
    pub self_retrieval: SelfRetrievalReport,
}

pub struct DedupReport {
    pub duplicate_groups: usize,
    pub duplicate_count: i64,
    pub duplicate_rate: f64,
    pub worst_groups: Vec<(String, i64)>,
}

pub struct ProjectLeakReport {
    pub total_tested: usize,
    pub leaked: usize,
    pub leak_rate: f64,
}

pub struct TitleQualityReport {
    pub total: i64,
    pub bullet_prefix: i64,
    pub too_long: i64,
    pub bullet_rate: f64,
}

pub struct SelfRetrievalReport {
    pub total_tested: usize,
    pub found: usize,
    pub retrieval_rate: f64,
}
