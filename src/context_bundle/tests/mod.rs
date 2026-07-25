mod executor;
mod planner;
mod schema;

use super::domain::{
    AgentRole, ChannelKind, ContextItem, ContextRequest, ItemValidity, ProjectRef, RiskClass,
    SourceKind, TrustClass, CONTEXT_BUNDLE_SCHEMA_VERSION,
};

pub(super) fn request() -> ContextRequest {
    ContextRequest {
        schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
        task: "fix startup migration races".to_string(),
        project: ProjectRef {
            key: "demo/project".to_string(),
        },
        branch: Some("main".to_string()),
        worktree: None,
        role: AgentRole::Coder,
        as_of_epoch: 1_710_000_000,
        token_budget: 3_000,
        risk: RiskClass::Low,
        include_superseded: false,
    }
}

pub(super) fn item(stable_key: &str, channel: ChannelKind, title: &str, text: &str) -> ContextItem {
    ContextItem {
        stable_key: stable_key.to_string(),
        channel,
        title: title.to_string(),
        text: text.to_string(),
        source_kind: SourceKind::Canonical,
        canonical_ref: Some(format!("{stable_key}@canonical")),
        projection_ref: None,
        evidence_refs: Vec::new(),
        validity: ItemValidity::Current,
        trust: TrustClass::Standard,
        project: Some("demo/project".to_string()),
        branch: None,
    }
}
