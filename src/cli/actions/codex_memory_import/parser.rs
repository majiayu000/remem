//! Versioned parser for the closed format set (GH-852 B-006). Only formats
//! fingerprinted against a real Codex CLI installation are accepted; anything
//! else fails the whole batch. Host content is data — header values are never
//! interpreted as commands, config, or writable paths.

use anyhow::{bail, Result};

pub(super) const FORMAT_ID: &str = "codex-rollout-summary";
pub(super) const FORMAT_VERSION: &str = "v1";

/// Canonical external record parsed from one rollout-summary file.
#[derive(Debug, Clone)]
pub(super) struct CodexRecord {
    pub rel_id: String,
    /// Host-provided workspace evidence (absolute path string; treated as
    /// data until verified against the local filesystem).
    pub cwd: String,
    pub body: String,
}

/// codex-rollout-summary/v1 header, verified on codex-cli 0.145.0:
/// `thread_id`, `updated_at` (RFC3339 with offset), `rollout_path` (absolute),
/// `cwd` (absolute), optional `git_branch`; terminated by a blank line.
pub(super) fn parse_rollout_summary(rel_id: &str, content: &str) -> Result<CodexRecord> {
    let mut lines = content.lines();
    let mut header: Vec<(String, String)> = Vec::new();
    let mut header_len = 0usize;
    for line in lines.by_ref() {
        header_len += line.len() + 1;
        if line.trim().is_empty() {
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            bail!("codex memories record {rel_id} has a malformed header line");
        };
        header.push((key.trim().to_string(), value.trim().to_string()));
    }

    let keys: Vec<&str> = header.iter().map(|(key, _)| key.as_str()).collect();
    let known_v1 = keys == ["thread_id", "updated_at", "rollout_path", "cwd"]
        || keys
            == [
                "thread_id",
                "updated_at",
                "rollout_path",
                "cwd",
                "git_branch",
            ];
    if !known_v1 {
        bail!(
            "codex memories record {rel_id} does not match the {FORMAT_ID}/{FORMAT_VERSION} \
             header fingerprint; refusing to guess a compatible format"
        );
    }

    let value_of = |name: &str| -> &str {
        header
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .unwrap_or("")
    };
    let thread_id = value_of("thread_id");
    let updated_at = value_of("updated_at");
    let rollout_path = value_of("rollout_path");
    let cwd = value_of("cwd");

    if thread_id.is_empty() {
        bail!("codex memories record {rel_id} has an empty thread_id");
    }
    if chrono::DateTime::parse_from_rfc3339(updated_at).is_err() {
        bail!("codex memories record {rel_id} has a non-RFC3339 updated_at value");
    }
    if !rollout_path.starts_with('/') {
        bail!("codex memories record {rel_id} has a non-absolute rollout_path");
    }
    if !cwd.starts_with('/') {
        bail!("codex memories record {rel_id} has a non-absolute cwd");
    }

    let body = content.get(header_len..).unwrap_or("").trim();
    if body.is_empty() {
        bail!("codex memories record {rel_id} has an empty body");
    }

    Ok(CodexRecord {
        rel_id: rel_id.to_string(),
        cwd: cwd.to_string(),
        body: body.to_string(),
    })
}
