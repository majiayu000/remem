use serde::Serialize;

use super::Memory;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryStalenessLabel {
    pub status: String,
    pub age: &'static str,
    pub source_anchor: &'static str,
    pub label: String,
}

pub fn memory_staleness_label(memory: &Memory, now_epoch: i64) -> MemoryStalenessLabel {
    let age = age_staleness(memory.updated_at_epoch, now_epoch);
    let source_anchor = "untracked";
    MemoryStalenessLabel {
        status: memory.status.clone(),
        age,
        source_anchor,
        label: format!(
            "status={}; staleness={age}; source_anchor={source_anchor}",
            memory.status
        ),
    }
}

pub fn memory_staleness(memory: &Memory, now_epoch: i64) -> String {
    format!(
        "status={}; staleness={}",
        memory.status,
        age_staleness(memory.updated_at_epoch, now_epoch)
    )
}

pub fn age_staleness_label(updated_at_epoch: i64, now_epoch: i64) -> String {
    format!("staleness={}", age_staleness(updated_at_epoch, now_epoch))
}

pub fn age_staleness(updated_at_epoch: i64, now_epoch: i64) -> &'static str {
    let age_days = now_epoch.saturating_sub(updated_at_epoch) / 86_400;
    if age_days <= 30 {
        "fresh"
    } else if age_days <= 90 {
        "aging"
    } else {
        "old"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(updated_at_epoch: i64, status: &str) -> Memory {
        Memory {
            id: 1,
            session_id: None,
            project: "/repo".to_string(),
            topic_key: None,
            title: "Staleness fixture".to_string(),
            text: "body".to_string(),
            memory_type: "decision".to_string(),
            files: None,
            created_at_epoch: updated_at_epoch,
            updated_at_epoch,
            status: status.to_string(),
            branch: None,
            scope: "project".to_string(),
        }
    }

    #[test]
    fn labels_memory_status_age_and_untracked_source_anchor() {
        let label = memory_staleness_label(&memory(1_700_000_000, "active"), 1_700_000_000);

        assert_eq!(label.status, "active");
        assert_eq!(label.age, "fresh");
        assert_eq!(label.source_anchor, "untracked");
        assert_eq!(
            label.label,
            "status=active; staleness=fresh; source_anchor=untracked"
        );
        assert_eq!(
            memory_staleness(&memory(1_700_000_000, "active"), 1_700_000_000),
            "status=active; staleness=fresh"
        );
    }

    #[test]
    fn classifies_age_buckets() {
        let now = 1_700_000_000;

        assert_eq!(age_staleness(now - 30 * 86_400, now), "fresh");
        assert_eq!(age_staleness(now - 31 * 86_400, now), "aging");
        assert_eq!(age_staleness(now - 91 * 86_400, now), "old");
    }
}
