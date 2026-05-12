use anyhow::{anyhow, Result};

/// Extraction task kinds. Priority order is encoded by the caller via the
/// `priority` column (lower number = higher priority); this enum carries
/// only the canonical name written into `extraction_tasks.task_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskKind {
    SessionRollup,
    ObservationExtract,
    MemoryCandidate,
    RuleCandidate,
    IndexUpdate,
}

impl TaskKind {
    pub fn as_db_value(self) -> &'static str {
        match self {
            TaskKind::SessionRollup => "session_rollup",
            TaskKind::ObservationExtract => "observation_extract",
            TaskKind::MemoryCandidate => "memory_candidate",
            TaskKind::RuleCandidate => "rule_candidate",
            TaskKind::IndexUpdate => "index_update",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "session_rollup" => Ok(TaskKind::SessionRollup),
            "observation_extract" => Ok(TaskKind::ObservationExtract),
            "memory_candidate" => Ok(TaskKind::MemoryCandidate),
            "rule_candidate" => Ok(TaskKind::RuleCandidate),
            "index_update" => Ok(TaskKind::IndexUpdate),
            other => Err(anyhow!("unknown extraction task_kind: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Processing,
    Delayed,
    Done,
    Failed,
}

impl TaskStatus {
    pub fn as_db_value(self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Processing => "processing",
            TaskStatus::Delayed => "delayed",
            TaskStatus::Done => "done",
            TaskStatus::Failed => "failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_kind_round_trip() {
        for k in [
            TaskKind::SessionRollup,
            TaskKind::ObservationExtract,
            TaskKind::MemoryCandidate,
            TaskKind::RuleCandidate,
            TaskKind::IndexUpdate,
        ] {
            assert_eq!(TaskKind::parse(k.as_db_value()).unwrap(), k);
        }
    }

    #[test]
    fn task_kind_rejects_unknown() {
        assert!(TaskKind::parse("nope").is_err());
        assert!(TaskKind::parse("").is_err());
    }
}
