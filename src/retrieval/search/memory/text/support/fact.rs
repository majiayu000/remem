use std::collections::HashSet;

use anyhow::Result;
use rusqlite::{types::ToSql, Connection};

#[derive(Default)]
pub(super) struct StructuredFactScope {
    pub(super) bound_ids: HashSet<i64>,
    pub(super) supported_ids: HashSet<i64>,
}

struct FactRow {
    memory_id: i64,
    subject: String,
    predicate: String,
    object: String,
    confidence: f64,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn structured_fact_scope(
    conn: &Connection,
    candidate_ids: &[i64],
    query_text: &str,
    explicit_entity_terms: &[String],
    claim_terms: &[String],
    min_confidence: f64,
    project: Option<&str>,
    mode: crate::retrieval::temporal::FactTimeMode,
) -> Result<StructuredFactScope> {
    if candidate_ids.is_empty()
        || !crate::retrieval::temporal::sqlite_table_exists(conn, "memory_facts")?
    {
        return Ok(StructuredFactScope::default());
    }

    let rows = load_fact_rows(conn, candidate_ids, project, mode)?;
    let binding_terms = query_binding_terms(query_text, explicit_entity_terms, &rows);
    let mut scope = StructuredFactScope::default();
    for row in rows {
        let exact_binding = !binding_terms.is_empty()
            && binding_terms.iter().all(|term| {
                [&row.subject, &row.object]
                    .iter()
                    .any(|value| super::super::super::claim::text_contains_exact_token(value, term))
            });

        let fact_text = format!(
            "{} {} {}",
            row.subject,
            row.predicate.replace('_', " "),
            row.object
        );
        if !exact_binding {
            continue;
        }
        scope.bound_ids.insert(row.memory_id);

        if row.confidence >= min_confidence
            && semantic_claims_match(&row, &fact_text, claim_terms)
            && predicate_compatible(query_text, &row.predicate)
        {
            scope.supported_ids.insert(row.memory_id);
        }
    }
    Ok(scope)
}

fn load_fact_rows(
    conn: &Connection,
    candidate_ids: &[i64],
    project: Option<&str>,
    mode: crate::retrieval::temporal::FactTimeMode,
) -> Result<Vec<FactRow>> {
    let placeholders = (1..=candidate_ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>();
    let mut conditions = vec![format!(
        "f.source_memory_id IN ({})",
        placeholders.join(", ")
    )];
    let mut params = candidate_ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn ToSql>)
        .collect::<Vec<_>>();
    let mut idx = params.len() + 1;
    if let Some(project) = project {
        conditions.push(format!("f.project = ?{idx}"));
        params.push(Box::new(project.to_string()));
        idx += 1;
    }
    let has_invalidated_at_epoch = crate::memory::facts::invalidated_at_epoch_available(conn)?;
    match mode {
        crate::retrieval::temporal::FactTimeMode::Current => {
            conditions.push(crate::memory::facts::current_fact_filter_sql(
                "f",
                has_invalidated_at_epoch,
            ));
            let now = chrono::Utc::now().timestamp();
            conditions.push(format!(
                "(f.valid_from_epoch IS NULL OR f.valid_from_epoch <= ?{idx})"
            ));
            conditions.push(format!(
                "(f.valid_to_epoch IS NULL OR f.valid_to_epoch > ?{idx})"
            ));
            params.push(Box::new(now));
        }
        crate::retrieval::temporal::FactTimeMode::AsOf(as_of_epoch) => {
            conditions.push(format!(
                "(f.valid_from_epoch IS NULL OR f.valid_from_epoch <= ?{idx})"
            ));
            conditions.push(crate::memory::facts::as_of_validity_filter_sql(
                "f",
                idx,
                has_invalidated_at_epoch,
            ));
            conditions.push(format!("f.learned_at_epoch <= ?{idx}"));
            if has_invalidated_at_epoch {
                conditions.push(format!(
                    "(f.invalidated_at_epoch IS NULL OR f.invalidated_at_epoch > ?{idx})"
                ));
            }
            params.push(Box::new(as_of_epoch));
        }
    }
    let sql = format!(
        "SELECT f.source_memory_id, f.subject, f.predicate, f.object, f.confidence
         FROM memory_facts f
         WHERE {}",
        conditions.join(" AND ")
    );
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok(FactRow {
            memory_id: row.get(0)?,
            subject: row.get(1)?,
            predicate: row.get(2)?,
            object: row.get(3)?,
            confidence: row.get(4)?,
        })
    })?;
    crate::db::query::collect_rows(rows)
}

fn query_binding_terms(
    query_text: &str,
    explicit_entity_terms: &[String],
    rows: &[FactRow],
) -> Vec<String> {
    let mut seen = HashSet::new();
    explicit_entity_terms
        .iter()
        .cloned()
        .chain(
            rows.iter()
                .flat_map(|row| [&row.subject, &row.object])
                .filter(|value| is_concrete_binding_value(value))
                .filter(|value| {
                    super::super::super::claim::text_contains_exact_token(query_text, value)
                })
                .map(|value| value.trim().to_string()),
        )
        .filter_map(|term| {
            let normalized = term.trim().to_lowercase();
            (!normalized.is_empty() && seen.insert(normalized)).then(|| term.trim().to_string())
        })
        .collect()
}

fn semantic_claims_match(row: &FactRow, fact_text: &str, claim_terms: &[String]) -> bool {
    claim_terms
        .iter()
        .filter(|term| !super::super::super::claim::is_nonsemantic_claim_modifier(term))
        .all(|term| {
            super::super::super::claim::claim_text_match_count(
                fact_text,
                std::slice::from_ref(term),
            ) > 0
                || (super::super::super::claim::is_relation_only_claim_term(term)
                    && predicate_compatible(term, &row.predicate))
        })
}

#[cfg(test)]
fn is_relation_only_claim_term(term: &str) -> bool {
    super::super::super::claim::is_relation_only_claim_term(term)
}

pub(super) fn relation_claim_matches_text(claim_term: &str, text: &str) -> bool {
    let claim_relations = super::super::super::claim::expressed_relation_kinds(claim_term);
    if claim_relations.len() != 1 {
        return false;
    }
    let text_relations = super::super::super::claim::expressed_relation_kinds(text);
    claim_relations
        .iter()
        .all(|relation| text_relations.contains(relation))
}

fn predicate_compatible(query_text: &str, predicate: &str) -> bool {
    let query_relations = super::super::super::claim::expressed_relation_kinds(query_text);
    if query_relations.len() > 1 {
        return false;
    }

    let predicate_relations = predicate_relation_kinds(predicate);
    if let Some(query_relation) = query_relations.iter().next() {
        return predicate_relations.contains(query_relation);
    }

    predicate_directly_expressed(query_text, predicate)
}

fn predicate_directly_expressed(query_text: &str, predicate: &str) -> bool {
    let normalized = predicate.trim().to_lowercase().replace('_', " ");
    let canonical = predicate
        .trim()
        .split('_')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    [normalized, canonical].into_iter().any(|term| {
        !term.is_empty()
            && super::super::super::claim::claim_text_match_count(query_text, &[term]) > 0
    })
}

fn predicate_relation_kinds(predicate: &str) -> HashSet<super::super::super::claim::RelationKind> {
    let normalized = predicate.trim().to_lowercase().replace('_', " ");
    let mut relations = super::super::super::claim::expressed_relation_kinds(&normalized);
    if relations.contains(&super::super::super::claim::RelationKind::Maintain) {
        relations.insert(super::super::super::claim::RelationKind::Own);
    }
    relations
}

fn is_concrete_binding_value(value: &str) -> bool {
    let normalized = value.trim().to_lowercase();
    normalized.chars().any(char::is_alphanumeric)
        && normalized.chars().count() >= 3
        && !matches!(
            normalized.as_str(),
            "active"
                | "closed"
                | "current"
                | "false"
                | "inactive"
                | "none"
                | "null"
                | "open"
                | "true"
                | "unknown"
        )
}

#[cfg(test)]
mod predicate_tests {
    use super::{is_relation_only_claim_term, predicate_compatible, query_binding_terms, FactRow};

    #[test]
    fn binding_terms_exclude_general_title_case_query_candidates() {
        let rows = [FactRow {
            memory_id: 1,
            subject: "HarborMint".to_string(),
            predicate: "maintains".to_string(),
            object: "Toma Reed".to_string(),
            confidence: 0.9,
        }];

        assert_eq!(
            query_binding_terms(
                "Who Maintains HarborMint?",
                &["HarborMint".to_string()],
                &rows
            ),
            vec!["HarborMint"]
        );
    }

    #[test]
    fn canonical_maintains_predicate_accepts_english_and_cjk_queries() {
        assert!(predicate_compatible(
            "Who Maintains HarborMint?",
            "maintains"
        ));
        assert!(predicate_compatible("谁维护港湾服务？", "maintains"));
        assert!(predicate_compatible("Who Rotates HarborMint?", "rotates"));
    }

    #[test]
    fn cjk_relation_aliases_match_only_compatible_predicate_families() {
        for (query, predicate) in [
            ("谁验证了港湾服务？", "verified_by"),
            ("谁维护港湾服务？", "maintains"),
            ("谁使用港湾服务？", "uses_command"),
            ("谁负责港湾服务？", "maintains"),
            ("谁拥有港湾服务？", "owned_by"),
            ("谁阻塞了港湾服务？", "blocked_by"),
            ("谁修复了港湾服务？", "fixed_by"),
            ("谁影响了港湾服务？", "affects_project"),
            ("谁替代了港湾服务？", "supersedes"),
        ] {
            assert!(
                predicate_compatible(query, predicate),
                "query={query} predicate={predicate}"
            );
        }
        assert!(!predicate_compatible("谁删除了港湾服务？", "verified_by"));
        assert!(!predicate_compatible("谁拥有港湾服务？", "affects_project"));
    }

    #[test]
    fn predicate_compatibility_rejects_unknown_or_conflicting_relations() {
        assert!(!predicate_compatible(
            "Who deleted HarborMint?",
            "verified_by"
        ));
        assert!(!predicate_compatible("谁删除了港湾服务？", "verified_by"));
        assert!(!predicate_compatible(
            "Who verified and deleted HarborMint?",
            "verified_by"
        ));
        assert!(!predicate_compatible(
            "Who owns HarborMint?",
            "affects_project"
        ));
        assert!(!predicate_compatible(
            "Who frobnicated HarborMint?",
            "verified_by"
        ));
    }

    #[test]
    fn predicate_fallback_accepts_only_pure_relation_claim_terms() {
        assert!(is_relation_only_claim_term("负责"));
        assert!(is_relation_only_claim_term("verified"));
        assert!(is_relation_only_claim_term("signs"));
        assert!(!is_relation_only_claim_term("欧洲负责环境"));
        assert!(!is_relation_only_claim_term("verified HarborMint"));
    }
}
