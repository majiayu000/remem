#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SectionKind {
    Preferences,
    Core,
    Workstreams,
    MemoryIndex,
    Sessions,
    RetrievalHints,
}

#[derive(Debug, Clone)]
pub(super) struct SectionPolicy {
    pub kind: SectionKind,
    pub item_limit: usize,
    pub char_limit: usize,
    pub include_types: Vec<&'static str>,
    pub exclude_types: Vec<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ContextLimits {
    pub total_char_limit: usize,
    pub candidate_fetch_limit: usize,
    pub memory_index_limit: usize,
    pub memory_index_char_limit: usize,
    pub core_item_limit: usize,
    pub core_char_limit: usize,
    pub session_limit: usize,
    pub self_diagnostic_limit: usize,
    pub preference_project_limit: usize,
    pub preference_global_limit: usize,
    pub preference_char_limit: usize,
}

impl Default for ContextLimits {
    fn default() -> Self {
        Self {
            total_char_limit: 12_000,
            candidate_fetch_limit: 120,
            memory_index_limit: 50,
            memory_index_char_limit: 4_000,
            core_item_limit: 6,
            core_char_limit: 3_000,
            session_limit: 5,
            self_diagnostic_limit: 2,
            preference_project_limit: 20,
            preference_global_limit: 10,
            preference_char_limit: 1_500,
        }
    }
}

impl ContextLimits {
    pub(super) fn from_env() -> Self {
        Self::from_env_reader(|key| std::env::var(key).ok())
    }

    pub(super) fn from_env_reader<F>(mut read: F) -> Self
    where
        F: FnMut(&str) -> Option<String>,
    {
        let defaults = Self::default();
        Self {
            total_char_limit: read_usize(
                &mut read,
                "REMEM_CONTEXT_TOTAL_CHAR_LIMIT",
                defaults.total_char_limit,
            ),
            candidate_fetch_limit: read_usize(
                &mut read,
                "REMEM_CONTEXT_CANDIDATE_FETCH_LIMIT",
                defaults.candidate_fetch_limit,
            ),
            memory_index_limit: read_usize_with_alias(
                &mut read,
                "REMEM_CONTEXT_MEMORY_INDEX_LIMIT",
                "REMEM_CONTEXT_OBSERVATIONS",
                defaults.memory_index_limit,
            ),
            memory_index_char_limit: read_usize(
                &mut read,
                "REMEM_CONTEXT_MEMORY_INDEX_CHAR_LIMIT",
                defaults.memory_index_char_limit,
            ),
            core_item_limit: read_usize(
                &mut read,
                "REMEM_CONTEXT_CORE_ITEM_LIMIT",
                defaults.core_item_limit,
            ),
            core_char_limit: read_usize(
                &mut read,
                "REMEM_CONTEXT_CORE_CHAR_LIMIT",
                defaults.core_char_limit,
            ),
            session_limit: read_usize(
                &mut read,
                "REMEM_CONTEXT_SESSION_COUNT",
                defaults.session_limit,
            ),
            self_diagnostic_limit: read_usize(
                &mut read,
                "REMEM_CONTEXT_SELF_DIAGNOSTIC_LIMIT",
                defaults.self_diagnostic_limit,
            ),
            preference_project_limit: read_usize(
                &mut read,
                "REMEM_CONTEXT_PREFERENCE_PROJECT_LIMIT",
                defaults.preference_project_limit,
            ),
            preference_global_limit: read_usize(
                &mut read,
                "REMEM_CONTEXT_PREFERENCE_GLOBAL_LIMIT",
                defaults.preference_global_limit,
            ),
            preference_char_limit: read_usize(
                &mut read,
                "REMEM_CONTEXT_PREFERENCE_CHAR_LIMIT",
                defaults.preference_char_limit,
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ContextPolicy {
    pub limits: ContextLimits,
    pub sections: Vec<SectionPolicy>,
}

impl ContextPolicy {
    pub(super) fn from_env() -> Self {
        Self::from_limits(ContextLimits::from_env())
    }

    pub(super) fn from_limits(limits: ContextLimits) -> Self {
        Self {
            limits,
            sections: vec![
                SectionPolicy {
                    kind: SectionKind::Preferences,
                    item_limit: limits.preference_project_limit + limits.preference_global_limit,
                    char_limit: limits.preference_char_limit,
                    include_types: vec!["preference"],
                    exclude_types: vec![],
                },
                SectionPolicy {
                    kind: SectionKind::Core,
                    item_limit: limits.core_item_limit,
                    char_limit: limits.core_char_limit,
                    include_types: vec!["bugfix", "architecture", "decision", "discovery"],
                    exclude_types: vec!["preference", "session_activity"],
                },
                SectionPolicy {
                    kind: SectionKind::Workstreams,
                    item_limit: 5,
                    char_limit: 1_200,
                    include_types: vec![],
                    exclude_types: vec![],
                },
                SectionPolicy {
                    kind: SectionKind::MemoryIndex,
                    item_limit: limits.memory_index_limit,
                    char_limit: limits.memory_index_char_limit,
                    include_types: vec![],
                    exclude_types: vec!["preference"],
                },
                SectionPolicy {
                    kind: SectionKind::Sessions,
                    item_limit: limits.session_limit,
                    char_limit: 2_200,
                    include_types: vec![],
                    exclude_types: vec![],
                },
                SectionPolicy {
                    kind: SectionKind::RetrievalHints,
                    item_limit: 1,
                    char_limit: 500,
                    include_types: vec![],
                    exclude_types: vec![],
                },
            ],
        }
    }

    pub(super) fn section(&self, kind: SectionKind) -> Option<&SectionPolicy> {
        self.sections.iter().find(|section| section.kind == kind)
    }

    pub(super) fn allows_memory_type(&self, kind: SectionKind, memory_type: &str) -> bool {
        let Some(section) = self.section(kind) else {
            return true;
        };
        if section.exclude_types.contains(&memory_type) {
            return false;
        }
        section.include_types.is_empty() || section.include_types.contains(&memory_type)
    }

    pub(super) fn section_item_limit(&self, kind: SectionKind, fallback: usize) -> usize {
        self.section(kind)
            .map(|section| section.item_limit)
            .unwrap_or(fallback)
    }

    pub(super) fn section_char_limit(&self, kind: SectionKind, fallback: usize) -> usize {
        self.section(kind)
            .map(|section| section.char_limit)
            .unwrap_or(fallback)
    }
}

fn read_usize<F>(read: &mut F, key: &str, default: usize) -> usize
where
    F: FnMut(&str) -> Option<String>,
{
    parse_usize(read(key)).unwrap_or(default)
}

fn read_usize_with_alias<F>(read: &mut F, key: &str, alias: &str, default: usize) -> usize
where
    F: FnMut(&str) -> Option<String>,
{
    parse_usize(read(key))
        .or_else(|| parse_usize(read(alias)))
        .unwrap_or(default)
}

fn parse_usize(value: Option<String>) -> Option<usize> {
    let parsed = value?.trim().parse::<usize>().ok()?;
    if parsed == 0 {
        None
    } else {
        Some(parsed)
    }
}
