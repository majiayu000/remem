//! Row scope for `doctor truth`: the memory rows the CurrentTruth projection
//! admits, narrowed to one subject's member claims when `--subject` is given.
//!
//! Lifecycle, supersedes and reference diagnostics filter memories by these
//! ids instead of re-deriving the scope from `memories.project` and
//! `topic_key`, so they follow the projection's alias set, repo owner and
//! target routing, owner-NULL fallback and non-global guard (#1136).

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::truth::{diagnostic_memory_admission, AdmittedMemory, TruthQuery};

pub(super) struct DoctorScope {
    /// JSON array of the canonical project and its active aliases.
    pub(super) projects_json: String,
    /// JSON array of the in-scope memory ids.
    pub(super) memory_ids_json: String,
    /// JSON array of the topic keys `--subject` selects for memory
    /// candidates: the selector plus every member claim's topic key.
    pub(super) candidate_topic_keys_json: Option<String>,
    /// Memory truth subject keys `--subject` selects; `None` keeps all.
    subjects: Option<BTreeSet<String>>,
}

impl DoctorScope {
    /// `query.subject_key` is ignored: admission does not depend on it.
    pub(super) fn load(
        conn: &Connection,
        query: &TruthQuery,
        subject: Option<&str>,
    ) -> Result<Self> {
        let (projects, admitted) = diagnostic_memory_admission(conn, query)
            .context("load the memory rows the CurrentTruth projection admits")?;
        let subjects = subject.map(|selector| {
            admitted
                .iter()
                .filter(|memory| selects(memory, selector))
                .map(|memory| memory.subject_key.clone())
                .collect::<BTreeSet<_>>()
        });
        let members = admitted
            .iter()
            .filter(|memory| {
                subjects
                    .as_ref()
                    .is_none_or(|keys| keys.contains(&memory.subject_key))
            })
            .collect::<Vec<_>>();
        let memory_ids = members.iter().map(|memory| memory.id).collect::<Vec<_>>();
        let candidate_topic_keys = subject.map(|selector| {
            let mut keys = members
                .iter()
                .filter_map(|memory| memory.topic_key.as_deref())
                .collect::<BTreeSet<_>>();
            keys.insert(selector);
            keys
        });
        Ok(Self {
            projects_json: serde_json::to_string(&projects)?,
            memory_ids_json: serde_json::to_string(&memory_ids)?,
            candidate_topic_keys_json: candidate_topic_keys
                .map(|keys| serde_json::to_string(&keys))
                .transpose()?,
            subjects,
        })
    }

    /// Whether a memory truth with this subject key is in scope.
    pub(super) fn keeps_subject(&self, subject_key: &str) -> bool {
        self.subjects
            .as_ref()
            .is_none_or(|keys| keys.contains(subject_key))
    }
}

/// A selector picks a memory truth by its exact subject key (for example a
/// full `state:` key) or by the topic key one of its member claims carries.
fn selects(memory: &AdmittedMemory, selector: &str) -> bool {
    memory.subject_key == selector || memory.topic_key.as_deref() == Some(selector)
}
