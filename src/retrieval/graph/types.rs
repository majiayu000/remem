#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphTraversalStatus {
    Ready,
    MissingTable,
    EmptyGraph,
    NoSeed,
    NoExpansion,
}

impl GraphTraversalStatus {
    pub const fn disabled_reason(self) -> Option<&'static str> {
        match self {
            Self::Ready => None,
            Self::MissingTable => Some("graph_edges table is unavailable"),
            Self::EmptyGraph => Some("graph_edges table is empty"),
            Self::NoSeed => Some("no eligible FTS/vector graph seeds"),
            Self::NoExpansion => Some("no eligible trusted graph expansion"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GraphPathKind {
    Supersedes,
    Mentions,
    TouchesFile,
}

impl GraphPathKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supersedes => "supersedes",
            Self::Mentions => "mentions",
            Self::TouchesFile => "touches_file",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphTraversalHit {
    pub memory_id: i64,
    pub hop_count: u8,
    pub path_kind: GraphPathKind,
    pub min_confidence: f64,
    pub seed_rank: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphTraversalLimits {
    pub max_seeds: usize,
    pub max_degree_per_node: usize,
    pub max_edges_scanned: usize,
    pub max_candidates: usize,
}

impl Default for GraphTraversalLimits {
    fn default() -> Self {
        Self {
            max_seeds: 32,
            max_degree_per_node: 64,
            max_edges_scanned: 2_048,
            max_candidates: 120,
        }
    }
}

impl GraphTraversalLimits {
    pub fn for_search(fetch_limit: i64) -> Self {
        Self {
            max_candidates: usize::try_from(fetch_limit.max(1)).unwrap_or(120),
            ..Self::default()
        }
    }

    pub(super) fn validate(self) -> anyhow::Result<()> {
        anyhow::ensure!(self.max_seeds > 0, "graph max_seeds must be positive");
        anyhow::ensure!(
            self.max_degree_per_node > 0,
            "graph max_degree_per_node must be positive"
        );
        anyhow::ensure!(
            self.max_edges_scanned > 0,
            "graph max_edges_scanned must be positive"
        );
        anyhow::ensure!(
            self.max_candidates > 0,
            "graph max_candidates must be positive"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GraphTraversalRequest<'a> {
    pub seed_memory_ids: &'a [i64],
    pub project: Option<&'a str>,
    pub memory_type: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub include_inactive: bool,
    pub reference_time_epoch: i64,
    pub limits: GraphTraversalLimits,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphTraversalDiagnostics {
    pub edges_scanned: usize,
    pub candidates_considered: usize,
    pub targets_filtered: usize,
    pub diagnostic_hint_edges: usize,
    pub extracted_from_edges: usize,
    pub ignored_trusted_edges: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphTraversalOutcome {
    pub status: GraphTraversalStatus,
    pub hits: Vec<GraphTraversalHit>,
    pub diagnostics: GraphTraversalDiagnostics,
}

impl GraphTraversalOutcome {
    pub(super) fn empty(status: GraphTraversalStatus) -> Self {
        Self {
            status,
            hits: Vec::new(),
            diagnostics: GraphTraversalDiagnostics::default(),
        }
    }
}
