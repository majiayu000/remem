//! Frozen import plan shared by dry-run and apply (GH-852 B-007, B-008,
//! B-017, B-018). Dry-run and apply call the same planning function; apply is
//! bound to the dry-run plan digest.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::discovery::DiscoveredFile;
use super::parser::{parse_rollout_summary, CodexRecord, FORMAT_ID, FORMAT_VERSION};

pub(super) const SOURCE_KIND: &str = "codex_native";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Classification {
    PendingReview,
    Quarantined,
    Dedup,
}

impl Classification {
    fn as_str(self) -> &'static str {
        match self {
            Self::PendingReview => "pending_review",
            Self::Quarantined => "quarantined",
            Self::Dedup => "dedup",
        }
    }
}

/// Destination route per B-017: verified workspace evidence maps to the
/// project; anything else lands in the Codex tool-owned search-only queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DestinationRoute {
    Project(String),
    ToolOwned,
}

impl DestinationRoute {
    pub(super) fn key(&self) -> String {
        match self {
            Self::Project(project) => format!("project:{project}"),
            Self::ToolOwned => "tool:codex-cli".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct PlanEntry {
    pub record: CodexRecord,
    pub identity: String,
    pub classification: Classification,
    pub route: DestinationRoute,
}

#[derive(Debug)]
pub(super) struct ImportPlan {
    pub entries: Vec<PlanEntry>,
    pub secret_blocked: usize,
    pub secret_blocked_files: Vec<String>,
    pub plan_digest: String,
}

impl ImportPlan {
    pub(super) fn source_state(&self) -> &'static str {
        if self.secret_blocked > 0 {
            "blocked"
        } else {
            "ready"
        }
    }

    pub(super) fn format_versions(&self) -> Vec<String> {
        vec![format!("{FORMAT_ID}/{FORMAT_VERSION}")]
    }

    pub(super) fn file_ids(&self) -> Vec<String> {
        if self.secret_blocked > 0 {
            return self.secret_blocked_files.clone();
        }
        self.entries
            .iter()
            .map(|entry| entry.record.rel_id.clone())
            .collect()
    }

    pub(super) fn planned_import(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.classification == Classification::PendingReview)
            .count()
    }

    pub(super) fn dedup(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.classification == Classification::Dedup)
            .count()
    }

    pub(super) fn quarantine(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.classification == Classification::Quarantined)
            .count()
    }
}

pub(super) fn build_plan(conn: &Connection, files: &[DiscoveredFile]) -> Result<ImportPlan> {
    // Parse everything first: any malformed record fails the whole batch
    // before any classification or hashing happens (B-006).
    let mut records = Vec::new();
    for file in files {
        records.push(parse_rollout_summary(&file.rel_id, &file.content)?);
    }

    // Pre-persistence secret boundary (B-018): if any record contains
    // secret-like content, the whole batch is blocked and no content hashes
    // are computed or persisted.
    let mut secret_blocked_files = Vec::new();
    for record in &records {
        if crate::adapter::redaction::redact_sensitive_text(record.body.as_str()) != record.body {
            secret_blocked_files.push(record.rel_id.clone());
        }
    }
    if !secret_blocked_files.is_empty() {
        return Ok(ImportPlan {
            entries: Vec::new(),
            secret_blocked: secret_blocked_files.len(),
            secret_blocked_files,
            plan_digest: String::new(),
        });
    }

    let mut entries = Vec::new();
    let mut seen_identities: HashSet<String> = HashSet::new();
    for record in records {
        let route = resolve_route(&record.cwd);
        let identity = record_identity(&record.body, &route);
        let topic_key = topic_key_for(&identity);

        let classification = if seen_identities.contains(&identity)
            || crate::memory_candidate::route::external_candidate_exists(
                conn,
                SOURCE_KIND,
                &topic_key,
                &record.body,
            )? {
            Classification::Dedup
        } else if crate::memory::poisoning::scan_instruction_pattern(&record.body).is_some() {
            Classification::Quarantined
        } else {
            Classification::PendingReview
        };
        seen_identities.insert(identity.clone());
        entries.push(PlanEntry {
            record,
            identity,
            classification,
            route,
        });
    }

    let plan_digest = digest_entries(&entries);
    Ok(ImportPlan {
        entries,
        secret_blocked: 0,
        secret_blocked_files: Vec::new(),
        plan_digest,
    })
}

/// B-017: the host-provided cwd counts as verified workspace evidence only if
/// it resolves to an existing local directory; the import command's own cwd is
/// never used.
fn resolve_route(record_cwd: &str) -> DestinationRoute {
    let path = Path::new(record_cwd);
    if path.is_absolute() && path.is_dir() {
        DestinationRoute::Project(crate::db::project_from_cwd(record_cwd))
    } else {
        DestinationRoute::ToolOwned
    }
}

/// Idempotent identity (B-008): bound to format id/version, canonical
/// (already secret-free) content, and destination route — not the file path,
/// because Codex may rename generated files.
fn record_identity(body: &str, route: &DestinationRoute) -> String {
    let mut hasher = Sha256::new();
    hasher.update(FORMAT_ID.as_bytes());
    hasher.update([0]);
    hasher.update(FORMAT_VERSION.as_bytes());
    hasher.update([0]);
    hasher.update(body.as_bytes());
    hasher.update([0]);
    hasher.update(route.key().as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(super) fn topic_key_for(identity: &str) -> String {
    format!("codex-native-{}", &identity[..identity.len().min(32)])
}

fn digest_entries(entries: &[PlanEntry]) -> String {
    let mut hasher = Sha256::new();
    for entry in entries {
        hasher.update(entry.record.rel_id.as_bytes());
        hasher.update([b'\t']);
        hasher.update(entry.identity.as_bytes());
        hasher.update([b'\t']);
        hasher.update(entry.classification.as_str().as_bytes());
        hasher.update([b'\t']);
        hasher.update(entry.route.key().as_bytes());
        hasher.update([b'\n']);
    }
    format!("{:x}", hasher.finalize())
}
