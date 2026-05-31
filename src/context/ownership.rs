#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct OwnerCounts {
    pub repo: usize,
    pub user: usize,
    pub workspace: usize,
    pub tool: usize,
    pub domain: usize,
    pub workstream: usize,
    pub session: usize,
    pub legacy: usize,
    pub unknown: usize,
}

impl OwnerCounts {
    pub(super) fn add_scope(&mut self, scope: Option<&str>) {
        match scope.map(str::trim).filter(|scope| !scope.is_empty()) {
            Some("repo") => self.repo += 1,
            Some("user") => self.user += 1,
            Some("workspace") => self.workspace += 1,
            Some("tool") => self.tool += 1,
            Some("domain") => self.domain += 1,
            Some("workstream") => self.workstream += 1,
            Some("session") => self.session += 1,
            None => self.legacy += 1,
            Some(_) => self.unknown += 1,
        }
    }

    pub(super) fn add_repo(&mut self, count: usize) {
        self.repo += count;
    }

    pub(super) fn add_user(&mut self, count: usize) {
        self.user += count;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OwnerMetadata {
    pub source_project: Option<String>,
    pub target_project: Option<String>,
    pub owner_scope: Option<String>,
    pub owner_key: Option<String>,
    pub topic_domain: Option<String>,
    pub context_class: Option<String>,
}

impl OwnerMetadata {
    pub(super) fn from_memory_row(
        row: &rusqlite::Row<'_>,
        offset: usize,
    ) -> rusqlite::Result<Self> {
        Ok(Self {
            source_project: row.get(offset)?,
            target_project: row.get(offset + 1)?,
            owner_scope: row.get(offset + 2)?,
            owner_key: row.get(offset + 3)?,
            topic_domain: row.get(offset + 4)?,
            context_class: row.get(offset + 5)?,
        })
    }

    fn scope(&self) -> Option<&str> {
        self.owner_scope.as_deref()
    }

    fn key(&self) -> Option<&str> {
        self.owner_key.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OwnerTrace {
    pub object_kind: &'static str,
    pub id: i64,
    pub title: String,
    pub source_project: Option<String>,
    pub target_project: Option<String>,
    pub owner_scope: Option<String>,
    pub owner_key: Option<String>,
    pub topic_domain: Option<String>,
    pub context_class: Option<String>,
    pub included: bool,
    pub reason: &'static str,
}

impl OwnerTrace {
    pub(super) fn memory(
        id: i64,
        title: &str,
        owner: &OwnerMetadata,
        included: bool,
        reason: &'static str,
    ) -> Self {
        Self {
            object_kind: "memory",
            id,
            title: title.to_string(),
            source_project: owner.source_project.clone(),
            target_project: owner.target_project.clone(),
            owner_scope: owner.owner_scope.clone(),
            owner_key: owner.owner_key.clone(),
            topic_domain: owner.topic_domain.clone(),
            context_class: owner.context_class.clone(),
            included,
            reason,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OwnerDecision {
    pub included: bool,
    pub reason: &'static str,
}

pub(super) fn startup_memory_owner_decision(
    current_project: &str,
    memory_project: &str,
    memory_scope: &str,
    owner: &OwnerMetadata,
) -> OwnerDecision {
    match owner.scope() {
        Some("repo")
            if owner.key() == Some(current_project)
                || owner.target_project.as_deref() == Some(current_project) =>
        {
            OwnerDecision {
                included: true,
                reason: "repo_owner_match",
            }
        }
        Some("workspace") => OwnerDecision {
            included: false,
            reason: "workspace_owner_not_enabled_for_startup",
        },
        Some("user") => OwnerDecision {
            included: false,
            reason: "user_memory_loaded_by_preference_layer",
        },
        Some("tool") => OwnerDecision {
            included: false,
            reason: "tool_not_relevant_to_startup",
        },
        Some("domain") => OwnerDecision {
            included: false,
            reason: "domain_not_relevant_to_startup",
        },
        Some("workstream") => OwnerDecision {
            included: false,
            reason: "workstream_memory_not_linked_to_startup",
        },
        Some("session") => OwnerDecision {
            included: false,
            reason: "session_memory_not_startup_durable",
        },
        Some(_) => OwnerDecision {
            included: false,
            reason: "unknown_owner_scope",
        },
        None if memory_project == current_project && memory_scope != "global" => OwnerDecision {
            included: true,
            reason: "legacy_project_fallback",
        },
        None => OwnerDecision {
            included: false,
            reason: "legacy_project_mismatch",
        },
    }
}
