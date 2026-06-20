use anyhow::{bail, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionTaskKind {
    SessionRollup,
    ObservationExtract,
    MemoryCandidate,
    UserContextCandidate,
    GraphCandidate,
    RuleCandidate,
    IndexUpdate,
}

impl ExtractionTaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionRollup => "session_rollup",
            Self::ObservationExtract => "observation_extract",
            Self::MemoryCandidate => "memory_candidate",
            Self::UserContextCandidate => "user_context_candidate",
            Self::GraphCandidate => "graph_candidate",
            Self::RuleCandidate => "rule_candidate",
            Self::IndexUpdate => "index_update",
        }
    }

    pub fn from_db(raw: &str) -> Result<Self> {
        match raw {
            "session_rollup" => Ok(Self::SessionRollup),
            "observation_extract" => Ok(Self::ObservationExtract),
            "memory_candidate" => Ok(Self::MemoryCandidate),
            "user_context_candidate" => Ok(Self::UserContextCandidate),
            "graph_candidate" => Ok(Self::GraphCandidate),
            "rule_candidate" => Ok(Self::RuleCandidate),
            "index_update" => Ok(Self::IndexUpdate),
            _ => bail!("unknown extraction task kind: {raw}"),
        }
    }

    pub(crate) fn priority(self) -> i64 {
        match self {
            Self::SessionRollup => 10,
            Self::ObservationExtract => 20,
            Self::UserContextCandidate => 30,
            Self::MemoryCandidate => 40,
            Self::GraphCandidate => 50,
            Self::RuleCandidate => 60,
            Self::IndexUpdate => 80,
        }
    }
}
