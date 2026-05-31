use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScopeObjectKind {
    Memory,
    Candidate,
    Workstream,
    SessionSummary,
}

impl ScopeObjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Candidate => "candidate",
            Self::Workstream => "workstream",
            Self::SessionSummary => "session-summary",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "memory" | "mem" => Some(Self::Memory),
            "candidate" | "memory-candidate" => Some(Self::Candidate),
            "workstream" | "ws" => Some(Self::Workstream),
            "session-summary" | "summary" => Some(Self::SessionSummary),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct ObjectRef {
    pub kind: ScopeObjectKind,
    pub id: i64,
}

impl ObjectRef {
    pub fn memory(id: i64) -> Self {
        Self {
            kind: ScopeObjectKind::Memory,
            id,
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        let Some((kind, id)) = value.split_once(':') else {
            bail!(
                "object ref must include a kind prefix, e.g. memory:123 or workstream:18: {value}"
            );
        };
        let kind = ScopeObjectKind::parse(kind.trim())
            .ok_or_else(|| anyhow!("unsupported object ref kind: {}", kind.trim()))?;
        let id = id
            .trim()
            .parse::<i64>()
            .with_context(|| format!("invalid object ref id: {value}"))?;
        if id <= 0 {
            bail!("object ref id must be positive: {value}");
        }
        Ok(Self { kind, id })
    }
}

impl fmt::Display for ObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind.as_str(), self.id)
    }
}

pub fn parse_object_refs(values: &[String]) -> Result<Vec<ObjectRef>> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        for token in value.split(|ch: char| ch.is_whitespace() || ch == ',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            let object_ref = ObjectRef::parse(token)?;
            if seen.insert(object_ref) {
                refs.push(object_ref);
            }
        }
    }
    Ok(refs)
}

pub fn memory_refs_from_ids(ids: &[i64]) -> Result<Vec<ObjectRef>> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        if *id <= 0 {
            bail!("memory id must be positive: {id}");
        }
        let object_ref = ObjectRef::memory(*id);
        if seen.insert(object_ref) {
            refs.push(object_ref);
        }
    }
    Ok(refs)
}
