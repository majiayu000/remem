use anyhow::Result;
use rusqlite::Connection;

use super::SchemaInvariant;

mod shape;
use shape::{
    require_index_columns, require_primary_key, require_restrict_foreign_key,
    require_sql_fragments, require_unique_columns,
};

pub(in crate::migrate) const V076_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::table(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "id",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "version",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "project",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "cluster_signature",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "member_ids_json",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "source_candidate_id",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "decision_kind",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "decision_ids_json",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "decision_payload_sha256",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "intended_superseded_ids_json",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "generated_topic_key",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "generated_memory_type",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "generated_title",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "generated_content",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "generated_field",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "pattern_id",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "pattern_version",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "source_operation",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "source_trust_class",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "occurrence_count",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "created_at_epoch",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts",
        "updated_at_epoch",
    ),
    SchemaInvariant::index(
        76,
        "dream_poisoning_quarantine",
        "idx_dream_quarantine_project_recent",
    ),
    SchemaInvariant::index(
        76,
        "dream_poisoning_quarantine",
        "idx_dream_quarantine_candidate",
    ),
    SchemaInvariant::trigger(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts_no_replace",
    ),
    SchemaInvariant::trigger(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts_initial_counters",
    ),
    SchemaInvariant::trigger(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts_validate_intended_insert",
    ),
    SchemaInvariant::trigger(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts_validate_intended_update",
    ),
    SchemaInvariant::trigger(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts_immutable_payload",
    ),
    SchemaInvariant::trigger(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts_monotonic_recurrence",
    ),
    SchemaInvariant::trigger(
        76,
        "dream_poisoning_quarantine",
        "dream_quarantine_artifacts_no_delete",
    ),
    SchemaInvariant::table(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
        "identity_sha256",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
        "candidate_id",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
        "source_kind",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
        "memory_type",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
        "semantic_discriminator_sha256",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
        "source_project",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
        "owner_scope",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
        "owner_key",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
        "target_project",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
        "topic_key",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
        "text_sha256",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
        "first_seen_epoch",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
        "last_seen_epoch",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities",
        "occurrence_count",
    ),
    SchemaInvariant::index(
        76,
        "dream_poisoning_quarantine",
        "idx_external_candidate_identities_candidate",
    ),
    SchemaInvariant::trigger(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities_no_replace",
    ),
    SchemaInvariant::trigger(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities_immutable_update",
    ),
    SchemaInvariant::trigger(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities_monotonic_recurrence",
    ),
    SchemaInvariant::trigger(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_identities_no_delete",
    ),
    SchemaInvariant::table(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_recurrences",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_recurrences",
        "id",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_recurrences",
        "identity_sha256",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_recurrences",
        "canonical_candidate_id",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_recurrences",
        "candidate_id",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_recurrences",
        "recurrence_kind",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_recurrences",
        "pattern_id",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_recurrences",
        "pattern_version",
    ),
    SchemaInvariant::column(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_recurrences",
        "occurred_at_epoch",
    ),
    SchemaInvariant::index(
        76,
        "dream_poisoning_quarantine",
        "idx_external_candidate_recurrences_identity_recent",
    ),
    SchemaInvariant::index(
        76,
        "dream_poisoning_quarantine",
        "idx_external_candidate_recurrences_candidate",
    ),
    SchemaInvariant::trigger(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_recurrences_validate_insert",
    ),
    SchemaInvariant::trigger(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_recurrences_immutable_update",
    ),
    SchemaInvariant::trigger(
        76,
        "dream_poisoning_quarantine",
        "external_candidate_recurrences_no_delete",
    ),
];

pub(in crate::migrate) fn v076_critical_shape_findings(conn: &Connection) -> Result<Vec<String>> {
    let mut findings = Vec::new();
    require_primary_key(conn, &mut findings, "dream_quarantine_artifacts", "id")?;
    require_primary_key(
        conn,
        &mut findings,
        "external_candidate_identities",
        "identity_sha256",
    )?;
    require_primary_key(conn, &mut findings, "external_candidate_recurrences", "id")?;
    require_unique_columns(
        conn,
        &mut findings,
        "dream_quarantine_artifacts",
        &["project", "cluster_signature", "source_candidate_id"],
    )?;

    for (table, from, target, to) in [
        (
            "dream_quarantine_artifacts",
            "source_candidate_id",
            "memory_candidates",
            "id",
        ),
        (
            "external_candidate_identities",
            "candidate_id",
            "memory_candidates",
            "id",
        ),
        (
            "external_candidate_recurrences",
            "identity_sha256",
            "external_candidate_identities",
            "identity_sha256",
        ),
        (
            "external_candidate_recurrences",
            "canonical_candidate_id",
            "memory_candidates",
            "id",
        ),
        (
            "external_candidate_recurrences",
            "candidate_id",
            "memory_candidates",
            "id",
        ),
    ] {
        require_restrict_foreign_key(conn, &mut findings, table, from, target, to)?;
    }

    for (index, columns) in [
        (
            "idx_dream_quarantine_project_recent",
            &["project", "updated_at_epoch", "id"][..],
        ),
        (
            "idx_dream_quarantine_candidate",
            &["source_candidate_id"][..],
        ),
        (
            "idx_external_candidate_identities_candidate",
            &["candidate_id"][..],
        ),
        (
            "idx_external_candidate_recurrences_identity_recent",
            &["identity_sha256", "id"][..],
        ),
        (
            "idx_external_candidate_recurrences_candidate",
            &["candidate_id"][..],
        ),
    ] {
        require_index_columns(conn, &mut findings, index, columns)?;
    }

    require_sql_fragments(
        conn,
        &mut findings,
        "table",
        "dream_quarantine_artifacts",
        &[
            "decision_kind in ('merge', 'no_merge', 'conflict')",
            "typeof(member_ids_json) = 'text'",
            "json_valid(member_ids_json)",
            "json_type(member_ids_json) = 'array'",
            "json_array_length(member_ids_json) > 0",
            "typeof(decision_ids_json) = 'text'",
            "json_valid(decision_ids_json)",
            "json_type(decision_ids_json) = 'array'",
            "typeof(decision_payload_sha256) = 'text'",
            "length(decision_payload_sha256) = 64",
            "decision_payload_sha256 not glob '*[^0-9a-f]*'",
            "typeof(intended_superseded_ids_json) = 'text'",
            "json_valid(intended_superseded_ids_json)",
            "json_type(intended_superseded_ids_json) = 'array'",
            "generated_field in ( 'dream.topic_key', 'dream.memory_type', 'dream.title', 'dream.content', 'dream.title_content'",
            "length(trim(pattern_id)) > 0",
            "pattern_version > 0",
            "source_operation = 'dream'",
            "source_trust_class = 'external_content'",
            "generated_topic_key is not null",
            "generated_memory_type is not null",
            "generated_title is not null",
            "generated_content is not null",
            "length(trim(generated_topic_key)) > 0",
            "length(trim(generated_memory_type)) > 0",
            "length(trim(generated_title)) > 0",
            "length(trim(generated_content)) > 0",
            "generated_topic_key is null",
            "generated_memory_type is null",
            "generated_title is null",
            "generated_content is null",
            "generated_field = 'dream.no_merge_reason'",
            "generated_field = 'dream.conflict_reason'",
            "occurrence_count >= 1",
            "version = occurrence_count",
            "updated_at_epoch >= created_at_epoch",
        ],
    )?;
    require_sql_fragments(
        conn,
        &mut findings,
        "table",
        "external_candidate_identities",
        &[
            "typeof(identity_sha256) = 'text'",
            "length(identity_sha256) = 64",
            "identity_sha256 not glob '*[^0-9a-f]*'",
            "typeof(memory_type) = 'text'",
            "length(trim(memory_type)) > 0",
            "semantic_discriminator_sha256 is null",
            "typeof(semantic_discriminator_sha256) = 'text'",
            "length(semantic_discriminator_sha256) = 64",
            "semantic_discriminator_sha256 not glob '*[^0-9a-f]*'",
            "typeof(text_sha256) = 'text'",
            "length(text_sha256) = 64",
            "text_sha256 not glob '*[^0-9a-f]*'",
            "occurrence_count >= 1",
            "last_seen_epoch >= first_seen_epoch",
        ],
    )?;
    require_sql_fragments(
        conn,
        &mut findings,
        "table",
        "external_candidate_recurrences",
        &[
            "'review_candidate'",
            "'discarded_pattern'",
            "'acknowledged_pattern'",
            "'terminal_duplicate'",
            "pattern_id is not null",
            "pattern_version is not null",
            "pattern_version > 0",
        ],
    )?;

    for (trigger, fragments) in [
        (
            "dream_quarantine_artifacts_no_replace",
            &[
                "before insert on dream_quarantine_artifacts",
                "id = new.id",
                "project = new.project",
                "cluster_signature = new.cluster_signature",
                "source_candidate_id = new.source_candidate_id",
                "artifact already exists",
            ][..],
        ),
        (
            "dream_quarantine_artifacts_initial_counters",
            &[
                "before insert on dream_quarantine_artifacts",
                "new.version != 1 or new.occurrence_count != 1",
                "counters must start at one",
            ][..],
        ),
        (
            "dream_quarantine_artifacts_validate_intended_insert",
            &[
                "before insert on dream_quarantine_artifacts",
                "json_each(new.member_ids_json)",
                "json_each(new.intended_superseded_ids_json)",
                "json_each(new.decision_ids_json)",
                "type != 'integer' or atom <= 0",
                "earlier.atom >= later.atom",
                "json_array_length(new.decision_ids_json) < 2",
                "json_array_length(new.decision_ids_json) != 0",
                "member.atom = intended.atom",
                "json(new.decision_ids_json) != json(new.intended_superseded_ids_json)",
                "member.atom = decision_id.atom",
                "raise(abort, 'invalid dream decision provenance')",
            ][..],
        ),
        (
            "dream_quarantine_artifacts_validate_intended_update",
            &[
                "before update of decision_kind, decision_ids_json, intended_superseded_ids_json, member_ids_json",
                "json_each(new.member_ids_json)",
                "json_each(new.intended_superseded_ids_json)",
                "json_each(new.decision_ids_json)",
                "earlier.atom >= later.atom",
                "json(new.decision_ids_json) != json(new.intended_superseded_ids_json)",
                "json_array_length(new.decision_ids_json) < 2",
                "member.atom = intended.atom",
                "member.atom = decision_id.atom",
            ][..],
        ),
        (
            "dream_quarantine_artifacts_immutable_payload",
            &[
                "before update of id, project, cluster_signature, member_ids_json",
                "source_candidate_id, decision_kind, decision_ids_json",
                "decision_payload_sha256, intended_superseded_ids_json",
                "generated_topic_key, generated_memory_type",
                "generated_title, generated_content, generated_field, pattern_id",
                "pattern_version, source_operation, source_trust_class",
                "created_at_epoch",
                "payload is immutable",
            ][..],
        ),
        (
            "dream_quarantine_artifacts_monotonic_recurrence",
            &[
                "before update of version, occurrence_count, updated_at_epoch",
                "new.version != old.version + 1",
                "new.occurrence_count != old.occurrence_count + 1",
                "new.updated_at_epoch < old.updated_at_epoch",
                "recurrence is not monotonic",
            ][..],
        ),
        (
            "dream_quarantine_artifacts_no_delete",
            &[
                "before delete on dream_quarantine_artifacts",
                "artifacts cannot be deleted",
            ][..],
        ),
        (
            "external_candidate_identities_no_replace",
            &[
                "before insert on external_candidate_identities",
                "when exists",
                "identity_sha256 = new.identity_sha256",
                "external candidate identity already exists",
            ][..],
        ),
        (
            "external_candidate_identities_immutable_update",
            &[
                "before update of identity_sha256, candidate_id, source_kind, memory_type",
                "semantic_discriminator_sha256, source_project, owner_scope, owner_key",
                "target_project, topic_key, text_sha256, first_seen_epoch",
                "external candidate identity fields are immutable",
            ][..],
        ),
        (
            "external_candidate_identities_monotonic_recurrence",
            &[
                "before update of last_seen_epoch, occurrence_count",
                "new.last_seen_epoch < old.last_seen_epoch",
                "new.occurrence_count != old.occurrence_count + 1",
                "identity recurrence is not monotonic",
            ][..],
        ),
        (
            "external_candidate_identities_no_delete",
            &[
                "before delete on external_candidate_identities",
                "ledger entries cannot be deleted",
            ][..],
        ),
        (
            "external_candidate_recurrences_validate_insert",
            &[
                "before insert on external_candidate_recurrences",
                "candidate_id = new.canonical_candidate_id",
                "canonical identity mismatch",
            ][..],
        ),
        (
            "external_candidate_recurrences_immutable_update",
            &[
                "before update on external_candidate_recurrences",
                "recurrence is immutable",
            ][..],
        ),
        (
            "external_candidate_recurrences_no_delete",
            &[
                "before delete on external_candidate_recurrences",
                "recurrences cannot be deleted",
            ][..],
        ),
    ] {
        require_sql_fragments(conn, &mut findings, "trigger", trigger, fragments)?;
    }
    Ok(findings)
}
