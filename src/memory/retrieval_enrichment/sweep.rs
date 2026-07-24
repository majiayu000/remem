//! Bounded, non-blocking idle worker sweep for retrieval enrichment (GH-850).
//!
//! Durable conditional claim/lease/attempt before any AI call, hard timeout
//! below the lease, and success/failure both committed through the same
//! source/generator/security/attempt/lease CAS so stale or late outcomes
//! affect zero rows.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, TransactionBehavior};

use super::{
    build_prompt, build_search_context, compose_search_context, ensure_retrieval_open,
    load_snapshot, parse_enrichment_output, sanitize_enrichment, EnrichmentErrorCode,
    EnrichmentSnapshot, DUE_PREDICATE_SQL, ELIGIBLE_STATUS_SQL, ENRICHMENT_HARD_TIMEOUT_SECS,
    ENRICHMENT_LEASE_SECS, ENRICHMENT_SYSTEM_PROMPT, IDLE_ENRICHMENT_BATCH_SIZE,
    RETRIEVAL_ENRICHMENT_SECURITY_POLICY_VERSION, RETRIEVAL_ENRICHMENT_VERSION,
};

/// One generation executor. Production uses the existing memory AI profile via
/// `ai::call_ai`; tests inject deterministic fakes.
pub(crate) trait EnrichmentGenerator {
    fn generate(
        &self,
        system_prompt: &str,
        user_message: &str,
    ) -> impl std::future::Future<Output = Result<String>>;
}

pub(crate) struct AiEnrichmentGenerator;

impl EnrichmentGenerator for AiEnrichmentGenerator {
    async fn generate(&self, system_prompt: &str, user_message: &str) -> Result<String> {
        crate::ai::call_ai(
            system_prompt,
            user_message,
            crate::ai::UsageContext {
                project: None,
                session_id: None,
                operation: "retrieval_enrichment",
                host: None,
                profile: None,
            },
        )
        .await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowOutcome {
    Ready,
    Failed,
    Stale,
    NotClaimed,
}

/// Idle sweep entry used by the worker after extraction tasks and the durable
/// job queue, before the embedding backfill. Returns true only when at least
/// one row became ready, so failure-only sweeps report no-work and `once`
/// mode cannot tight-loop.
pub(crate) async fn run_idle_retrieval_enrichment(owner: &str) -> Result<bool> {
    run_idle_sweep(&AiEnrichmentGenerator, owner, IDLE_ENRICHMENT_BATCH_SIZE).await
}

pub(crate) async fn run_idle_sweep<G: EnrichmentGenerator>(
    generator: &G,
    owner: &str,
    batch_size: i64,
) -> Result<bool> {
    let mut conn = crate::db::open_db()?;
    if let Err(error) = ensure_retrieval_open(&conn) {
        crate::log::error(
            "enrichment",
            &format!("idle enrichment sweep blocked: {error}"),
        );
        return Ok(false);
    }
    let candidates = select_due_candidates(&conn, batch_size)?;
    if candidates.is_empty() {
        return Ok(false);
    }
    let mut ready = 0usize;
    for memory_id in candidates {
        match process_one(&mut conn, generator, owner, memory_id).await? {
            RowOutcome::Ready => ready += 1,
            RowOutcome::Failed | RowOutcome::Stale | RowOutcome::NotClaimed => {}
        }
    }
    if ready > 0 {
        crate::log::info(
            "enrichment",
            &format!(
                "idle sweep enriched {ready} memory row(s) \
                 (generator=v{RETRIEVAL_ENRICHMENT_VERSION} \
                 policy=v{RETRIEVAL_ENRICHMENT_SECURITY_POLICY_VERSION})"
            ),
        );
    }
    Ok(ready > 0)
}

pub(crate) fn select_due_candidates(conn: &Connection, batch_size: i64) -> Result<Vec<i64>> {
    let sql = format!(
        "SELECT id FROM memories
         WHERE {ELIGIBLE_STATUS_SQL} AND {DUE_PREDICATE_SQL}
         ORDER BY COALESCE(search_context_next_retry_at_epoch, 0), updated_at_epoch, id
         LIMIT ?4"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![
            RETRIEVAL_ENRICHMENT_VERSION,
            RETRIEVAL_ENRICHMENT_SECURITY_POLICY_VERSION,
            chrono::Utc::now().timestamp(),
            batch_size.max(0)
        ],
        |row| row.get::<_, i64>(0),
    )?;
    crate::db::query::collect_rows(rows)
}

pub(crate) struct ClaimedRow {
    pub(crate) snapshot: EnrichmentSnapshot,
    pub(crate) attempt: i64,
}

/// Durable conditional claim inside one short `BEGIN IMMEDIATE` transaction.
/// Only after commit may any external call happen; a concurrent loser affects
/// zero rows and must not call the AI.
pub(crate) fn claim_row(
    conn: &mut Connection,
    owner: &str,
    memory_id: i64,
) -> Result<Option<ClaimedRow>> {
    let now = chrono::Utc::now().timestamp();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let claimed = tx.execute(
        &format!(
            "UPDATE memories SET
                search_context_enrichment_attempt = search_context_enrichment_attempt + 1,
                search_context_lease_owner = ?4,
                search_context_lease_expires_at_epoch = ?3 + ?5,
                search_context_claimed_enrichment_version = ?1,
                search_context_claimed_security_policy_version = ?2
             WHERE id = ?6 AND {ELIGIBLE_STATUS_SQL} AND {DUE_PREDICATE_SQL}"
        ),
        params![
            RETRIEVAL_ENRICHMENT_VERSION,
            RETRIEVAL_ENRICHMENT_SECURITY_POLICY_VERSION,
            now,
            owner,
            ENRICHMENT_LEASE_SECS,
            memory_id
        ],
    )?;
    if claimed == 0 {
        return Ok(None);
    }
    let Some(snapshot) = load_snapshot(&tx, memory_id)? else {
        // Row vanished between select and claim; nothing to enrich.
        return Ok(None);
    };
    tx.execute(
        "UPDATE memories SET search_context_claimed_source_hash = ?1
         WHERE id = ?2 AND search_context_lease_owner = ?3",
        params![snapshot.source_hash, memory_id, owner],
    )?;
    let attempt: i64 = tx.query_row(
        "SELECT search_context_enrichment_attempt FROM memories WHERE id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    tx.commit()?;
    Ok(Some(ClaimedRow { snapshot, attempt }))
}

pub(crate) async fn process_one<G: EnrichmentGenerator>(
    conn: &mut Connection,
    generator: &G,
    owner: &str,
    memory_id: i64,
) -> Result<RowOutcome> {
    let Some(claimed) = claim_row(conn, owner, memory_id)? else {
        return Ok(RowOutcome::NotClaimed);
    };
    let snapshot = &claimed.snapshot;
    let user_message = build_prompt(snapshot);

    let generated = match tokio::time::timeout(
        std::time::Duration::from_secs(ENRICHMENT_HARD_TIMEOUT_SECS),
        generator.generate(ENRICHMENT_SYSTEM_PROMPT, &user_message),
    )
    .await
    {
        Err(_elapsed) => {
            return record_failure(conn, owner, &claimed, EnrichmentErrorCode::AiTimeout, None);
        }
        Ok(Err(error)) => {
            return record_failure(
                conn,
                owner,
                &claimed,
                EnrichmentErrorCode::AiCallFailed,
                Some(&error),
            );
        }
        Ok(Ok(text)) => text,
    };

    let validated = match parse_enrichment_output(&generated) {
        Ok(validated) => validated,
        Err(error) => {
            return record_failure(
                conn,
                owner,
                &claimed,
                EnrichmentErrorCode::OutputRejected,
                Some(&error),
            );
        }
    };
    let validated = match sanitize_enrichment(validated) {
        Ok(validated) => validated,
        Err(error) => {
            return record_failure(
                conn,
                owner,
                &claimed,
                EnrichmentErrorCode::SecurityRejected,
                Some(&error),
            );
        }
    };

    let deterministic = build_search_context(
        &snapshot.memory_type,
        snapshot.topic_key.as_deref(),
        &snapshot.content,
        snapshot.files.as_deref(),
    );
    let composed = compose_search_context(&deterministic, &validated);

    // Prepare the embedding for the proposed authoritative passage outside
    // any transaction. provider=off is an explicit branch, never a fake vector.
    let prepared_vector = match prepare_index_embedding(snapshot, &composed) {
        Ok(prepared) => prepared,
        Err(error) => {
            return record_failure(
                conn,
                owner,
                &claimed,
                EnrichmentErrorCode::EmbeddingFailed,
                Some(&error),
            );
        }
    };

    commit_success(conn, owner, &claimed, &composed, prepared_vector.as_ref())
}

pub(crate) struct PreparedIndexVector {
    pub(crate) model: String,
    pub(crate) values: Vec<f32>,
    pub(crate) index_hash: String,
}

fn prepare_index_embedding(
    snapshot: &EnrichmentSnapshot,
    composed_search_context: &str,
) -> Result<Option<PreparedIndexVector>> {
    if crate::retrieval::embedding::provider_disabled_or_error()? {
        return Ok(None);
    }
    let embedding = match crate::retrieval::embedding::embed_memory_index(
        &snapshot.title,
        &snapshot.content,
        &snapshot.memory_type,
        snapshot.topic_key.as_deref(),
        composed_search_context,
    ) {
        Ok(embedding) => embedding,
        Err(error) if crate::retrieval::embedding::is_embedding_provider_off_error(&error) => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let index_hash = crate::retrieval::embedding::memory_index_hash(
        &snapshot.title,
        &snapshot.content,
        &snapshot.memory_type,
        snapshot.topic_key.as_deref(),
        composed_search_context,
    );
    Ok(Some(PreparedIndexVector {
        model: embedding.model().to_string(),
        values: embedding.values().to_vec(),
        index_hash,
    }))
}

const IDENTITY_CAS_WHERE_SQL: &str = "id = ?1
      AND search_context_lease_owner = ?2
      AND search_context_enrichment_attempt = ?3
      AND search_context_claimed_source_hash = ?4
      AND search_context_claimed_enrichment_version = ?5
      AND search_context_claimed_security_policy_version = ?6";

/// Success commit: single conditional CAS on the full claim identity plus a
/// live source-hash recheck inside the same transaction. A stale outcome
/// (source changed, lease taken over, newer attempt ready) affects zero rows.
pub(crate) fn commit_success(
    conn: &mut Connection,
    owner: &str,
    claimed: &ClaimedRow,
    composed: &str,
    prepared_vector: Option<&PreparedIndexVector>,
) -> Result<RowOutcome> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let live = load_snapshot(&tx, claimed.snapshot.id)?;
    let live_matches = live
        .as_ref()
        .is_some_and(|live| live.source_hash == claimed.snapshot.source_hash);
    if !live_matches {
        drop(tx);
        log_stale(claimed, "success");
        return Ok(RowOutcome::Stale);
    }
    let index_hash = prepared_vector.map(|prepared| prepared.index_hash.as_str());
    let updated = tx.execute(
        &format!(
            "UPDATE memories SET
                search_context = ?7,
                search_context_enrichment_version = ?5,
                search_context_security_policy_version = ?6,
                search_context_source_hash = ?4,
                search_context_fallback_source_hash = ?4,
                search_context_index_hash = ?8,
                search_context_lease_owner = NULL,
                search_context_lease_expires_at_epoch = NULL,
                search_context_claimed_source_hash = NULL,
                search_context_claimed_enrichment_version = NULL,
                search_context_claimed_security_policy_version = NULL,
                search_context_failure_count = 0,
                search_context_next_retry_at_epoch = NULL,
                search_context_last_error_code = NULL
             WHERE {IDENTITY_CAS_WHERE_SQL} AND {ELIGIBLE_STATUS_SQL}"
        ),
        params![
            claimed.snapshot.id,
            owner,
            claimed.attempt,
            claimed.snapshot.source_hash,
            RETRIEVAL_ENRICHMENT_VERSION,
            RETRIEVAL_ENRICHMENT_SECURITY_POLICY_VERSION,
            composed,
            index_hash,
        ],
    )?;
    if updated == 0 {
        drop(tx);
        log_stale(claimed, "success");
        return Ok(RowOutcome::Stale);
    }
    if let Some(prepared) = prepared_vector {
        crate::retrieval::vector::upsert_index_embedding(
            &tx,
            claimed.snapshot.id,
            &prepared.model,
            &prepared.index_hash,
            &prepared.values,
        )?;
    }
    tx.commit()?;
    crate::log::info(
        "enrichment",
        &format!(
            "memory id={} enriched (attempt={} generator=v{} policy=v{} source={} vector={})",
            claimed.snapshot.id,
            claimed.attempt,
            RETRIEVAL_ENRICHMENT_VERSION,
            RETRIEVAL_ENRICHMENT_SECURITY_POLICY_VERSION,
            super::hash_prefix(&claimed.snapshot.source_hash),
            prepared_vector.is_some(),
        ),
    );
    Ok(RowOutcome::Ready)
}

/// Failure commit through the exact same identity CAS. Only the owner of the
/// still-live claim may increase the failure count and set exponential
/// backoff (capped at 15 minutes); a late failure after takeover or ready
/// affects zero rows.
pub(crate) fn record_failure(
    conn: &mut Connection,
    owner: &str,
    claimed: &ClaimedRow,
    code: EnrichmentErrorCode,
    error: Option<&anyhow::Error>,
) -> Result<RowOutcome> {
    crate::log::error(
        "enrichment",
        &format!(
            "memory id={} enrichment failed (stage={} attempt={} generator=v{} policy=v{} source={})",
            claimed.snapshot.id,
            code.as_str(),
            claimed.attempt,
            RETRIEVAL_ENRICHMENT_VERSION,
            RETRIEVAL_ENRICHMENT_SECURITY_POLICY_VERSION,
            super::hash_prefix(&claimed.snapshot.source_hash),
        ),
    );
    if let Some(error) = error {
        let detail = crate::adapter::common::redact_sensitive_text(&format!("{error:#}"));
        crate::log::error(
            "enrichment",
            &format!(
                "memory id={} enrichment error detail: {}",
                claimed.snapshot.id,
                crate::db::truncate_str(&detail, 300)
            ),
        );
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let updated = tx.execute(
        &format!(
            "UPDATE memories SET
                search_context_failure_count = search_context_failure_count + 1,
                search_context_next_retry_at_epoch =
                    ?7 + MIN(900, 30 * (1 << MIN(search_context_failure_count, 5))),
                search_context_last_error_code = ?8,
                search_context_lease_owner = NULL,
                search_context_lease_expires_at_epoch = NULL,
                search_context_claimed_source_hash = NULL,
                search_context_claimed_enrichment_version = NULL,
                search_context_claimed_security_policy_version = NULL
             WHERE {IDENTITY_CAS_WHERE_SQL}"
        ),
        params![
            claimed.snapshot.id,
            owner,
            claimed.attempt,
            claimed.snapshot.source_hash,
            RETRIEVAL_ENRICHMENT_VERSION,
            RETRIEVAL_ENRICHMENT_SECURITY_POLICY_VERSION,
            chrono::Utc::now().timestamp(),
            code.as_str(),
        ],
    )?;
    tx.commit()
        .context("retrieval enrichment failure-state transaction failed")?;
    if updated == 0 {
        log_stale(claimed, "failure");
        return Ok(RowOutcome::Stale);
    }
    Ok(RowOutcome::Failed)
}

fn log_stale(claimed: &ClaimedRow, stage: &str) {
    crate::log::info(
        "enrichment",
        &format!(
            "memory id={} stale {stage} outcome ignored (attempt={} source={})",
            claimed.snapshot.id,
            claimed.attempt,
            super::hash_prefix(&claimed.snapshot.source_hash),
        ),
    );
}
