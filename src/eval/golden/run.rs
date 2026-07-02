use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};

use super::types::{
    CategoryEvaluation, EvaluationLayers, EvidenceRef, GoldenDataset, GoldenEvalReport,
    GoldenMemory, GoldenQuery, MetricSums, QueryEvaluation, QueryMetrics, QueryStatus,
};

const RANK_K: usize = 10;

#[derive(Default)]
pub(in crate::eval) struct CategoryAccumulator {
    total_queries: usize,
    abstention_queries: usize,
    abstention_passed: usize,
    query_tokens: usize,
    retrieval_latencies_ms: Vec<f64>,
    metrics: MetricSums,
}

pub fn load_dataset(dataset_path: &str) -> Result<GoldenDataset> {
    let content = std::fs::read_to_string(dataset_path)
        .with_context(|| format!("read golden eval dataset {dataset_path}"))?;
    let dataset: GoldenDataset = serde_json::from_str(&content)
        .with_context(|| format!("parse golden eval dataset {dataset_path}"))?;
    validate_dataset(&dataset)?;
    Ok(dataset)
}

pub fn run_dataset_path(
    conn: &Connection,
    dataset_path: &str,
    k: usize,
) -> Result<GoldenEvalReport> {
    let dataset = load_dataset(dataset_path)?;
    run_dataset(conn, &dataset, k)
}

pub fn run_dataset(
    conn: &Connection,
    dataset: &GoldenDataset,
    k: usize,
) -> Result<GoldenEvalReport> {
    if dataset.has_fixture_corpus() {
        evaluate_dataset_with_fixture_corpus(dataset, k)
    } else {
        evaluate_dataset(conn, dataset, k)
    }
}

pub fn evaluate_dataset_with_fixture_corpus(
    dataset: &GoldenDataset,
    k: usize,
) -> Result<GoldenEvalReport> {
    validate_dataset(dataset)?;
    let conn = Connection::open_in_memory().context("open in-memory golden eval DB")?;
    crate::migrate::run_migrations(&conn).context("migrate in-memory golden eval DB")?;
    seed_fixture_corpus(&conn, &dataset.corpus)?;
    evaluate_dataset(&conn, dataset, k)
}

pub fn evaluate_dataset(
    conn: &Connection,
    dataset: &GoldenDataset,
    k: usize,
) -> Result<GoldenEvalReport> {
    validate_dataset(dataset)?;
    let k = k.max(1);
    let fetch_limit = k.max(RANK_K) as i64;
    let mut query_reports = Vec::with_capacity(dataset.queries.len());
    let mut overall_sums = MetricSums::default();
    let mut categories = BTreeMap::<String, CategoryAccumulator>::new();
    let mut slices = BTreeMap::<String, CategoryAccumulator>::new();
    let mut skipped_queries = 0usize;
    let mut abstention_queries = 0usize;
    let mut abstention_passed = 0usize;

    for query in &dataset.queries {
        let started = Instant::now();
        let results = crate::retrieval::search::search_with_branch(
            conn,
            Some(&query.query),
            query.project.as_deref(),
            query.memory_type.as_deref(),
            fetch_limit,
            0,
            false,
            query.branch.as_deref(),
        )?;
        let retrieval_latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        let query_tokens = estimate_query_tokens(&query.query);
        let evaluation = evaluate_query(query, &results, k, query_tokens, retrieval_latency_ms);
        let category = categories.entry(query.category.clone()).or_default();
        record_bucket(category, query, &evaluation);
        let slice = slices.entry(query.slice_label().to_string()).or_default();
        record_bucket(slice, query, &evaluation);

        if query.expects_abstention() {
            abstention_queries += 1;
            if evaluation.status == QueryStatus::Pass {
                abstention_passed += 1;
            }
        } else if let Some(metrics) = evaluation.metrics.as_ref() {
            overall_sums.add(metrics);
        } else {
            skipped_queries += 1;
        }

        query_reports.push(evaluation);
    }

    Ok(GoldenEvalReport {
        evaluation_layers: EvaluationLayers::deterministic_retrieval_only(),
        version: dataset.version.clone(),
        description: dataset.description.clone(),
        k,
        rank_k: RANK_K,
        total_queries: dataset.queries.len(),
        scored_queries: overall_sums.averages().map_or(0, |metrics| metrics.count),
        skipped_queries,
        abstention_queries,
        abstention_passed,
        overall: overall_sums.averages(),
        by_slice: slices
            .into_iter()
            .map(|(name, slice)| (name, bucket_evaluation(slice)))
            .collect(),
        by_category: categories
            .into_iter()
            .map(|(name, category)| (name, bucket_evaluation(category)))
            .collect(),
        queries: query_reports,
    })
}

pub(in crate::eval) fn record_bucket(
    bucket: &mut CategoryAccumulator,
    query: &GoldenQuery,
    evaluation: &QueryEvaluation,
) {
    bucket.total_queries += 1;
    bucket.query_tokens += evaluation.query_tokens;
    bucket
        .retrieval_latencies_ms
        .push(evaluation.retrieval_latency_ms);
    if query.expects_abstention() {
        bucket.abstention_queries += 1;
        if evaluation.status == QueryStatus::Pass {
            bucket.abstention_passed += 1;
        }
    } else if let Some(metrics) = evaluation.metrics.as_ref() {
        bucket.metrics.add(metrics);
    }
}

pub(in crate::eval) fn bucket_evaluation(bucket: CategoryAccumulator) -> CategoryEvaluation {
    let query_tokens_per_query = if bucket.total_queries == 0 {
        0.0
    } else {
        bucket.query_tokens as f64 / bucket.total_queries as f64
    };
    let retrieval_latency_p50_ms = percentile(bucket.retrieval_latencies_ms.clone(), 50.0);
    let retrieval_latency_p95_ms = percentile(bucket.retrieval_latencies_ms, 95.0);
    CategoryEvaluation {
        total_queries: bucket.total_queries,
        scored_queries: bucket.metrics.averages().map_or(0, |m| m.count),
        abstention_queries: bucket.abstention_queries,
        abstention_passed: bucket.abstention_passed,
        query_tokens_per_query,
        retrieval_latency_p50_ms,
        retrieval_latency_p95_ms,
        metrics: bucket.metrics.averages(),
    }
}

fn validate_dataset(dataset: &GoldenDataset) -> Result<()> {
    if dataset.queries.is_empty() {
        return Err(anyhow!(
            "golden eval dataset must contain at least one query"
        ));
    }
    validate_fixture_corpus(&dataset.corpus)?;
    let mut seen_ids = HashSet::new();
    for query in &dataset.queries {
        if query.id.trim().is_empty() {
            return Err(anyhow!("golden eval query id must not be empty"));
        }
        if !seen_ids.insert(query.id.as_str()) {
            return Err(anyhow!("duplicate golden eval query id {}", query.id));
        }
        if query.query.trim().is_empty() {
            return Err(anyhow!(
                "golden eval query {} text must not be empty",
                query.id
            ));
        }
        if query.category.trim().is_empty() {
            return Err(anyhow!(
                "golden eval query {} category must not be empty",
                query.id
            ));
        }
        if query
            .slice
            .as_deref()
            .is_some_and(|slice| slice.trim().is_empty())
        {
            return Err(anyhow!(
                "golden eval query {} slice must not be empty",
                query.id
            ));
        }
        if query.expects_abstention()
            && (!query.evidence_refs.is_empty() || !query.relevant_ids.is_empty())
        {
            return Err(anyhow!(
                "golden eval query {} abstention case must not declare expected evidence",
                query.id
            ));
        }
        for evidence_ref in &query.evidence_refs {
            if !evidence_ref.has_match_criteria() {
                return Err(anyhow!(
                    "golden eval query {} contains an empty evidence ref",
                    query.id
                ));
            }
        }
    }
    validate_expected_refs_backed_by_corpus(dataset)?;
    Ok(())
}

fn validate_fixture_corpus(corpus: &[GoldenMemory]) -> Result<()> {
    let mut seen_topic_keys = HashSet::new();
    for (index, memory) in corpus.iter().enumerate() {
        let label = corpus_memory_label(index, memory);
        if memory.project.trim().is_empty() {
            return Err(anyhow!(
                "golden eval corpus memory {label} project must not be empty"
            ));
        }
        if memory.title.trim().is_empty() {
            return Err(anyhow!(
                "golden eval corpus memory {label} title must not be empty"
            ));
        }
        if memory.content.trim().is_empty() {
            return Err(anyhow!(
                "golden eval corpus memory {label} content must not be empty"
            ));
        }
        if memory.memory_type.trim().is_empty() {
            return Err(anyhow!(
                "golden eval corpus memory {label} memory_type must not be empty"
            ));
        }
        if memory.scope.trim().is_empty() {
            return Err(anyhow!(
                "golden eval corpus memory {label} scope must not be empty"
            ));
        }
        if !matches!(memory.status.as_str(), "active" | "stale" | "archived") {
            return Err(anyhow!(
                "golden eval corpus memory {label} status must be active, stale, or archived"
            ));
        }
        if memory
            .branch
            .as_deref()
            .is_some_and(|branch| branch.trim().is_empty())
        {
            return Err(anyhow!(
                "golden eval corpus memory {label} branch must not be empty"
            ));
        }
        if let Some(topic_key) = memory.topic_key.as_deref() {
            if topic_key.trim().is_empty() {
                return Err(anyhow!(
                    "golden eval corpus memory {label} topic_key must not be empty"
                ));
            }
            let key = (
                memory.project.clone(),
                memory.scope.clone(),
                topic_key.to_string(),
            );
            if !seen_topic_keys.insert(key) {
                return Err(anyhow!(
                    "duplicate golden eval corpus topic_key {} for project {} scope {}",
                    topic_key,
                    memory.project,
                    memory.scope
                ));
            }
        }
        if memory.access_count.is_some_and(|count| count < 0) {
            return Err(anyhow!(
                "golden eval corpus memory {label} access_count must be non-negative"
            ));
        }
        if memory.last_accessed_epoch.is_some_and(|epoch| epoch < 0) {
            return Err(anyhow!(
                "golden eval corpus memory {label} last_accessed_epoch must be non-negative"
            ));
        }
    }
    Ok(())
}

fn validate_expected_refs_backed_by_corpus(dataset: &GoldenDataset) -> Result<()> {
    if dataset.corpus.is_empty() {
        return Ok(());
    }

    let fixture_memories: Vec<_> = dataset
        .corpus
        .iter()
        .enumerate()
        .map(|(index, memory)| fixture_memory_for_validation(index, memory))
        .collect();

    for query in &dataset.queries {
        if query.expects_abstention() {
            continue;
        }
        if query.slice_label() == "associative" {
            super::validation::validate_associative_query(query, &fixture_memories)?;
        }
        for expected_ref in query.expected_refs() {
            let backed = fixture_memories.iter().any(|memory| {
                corpus_memory_matches_query_filter(memory, query) && expected_ref.matches(memory)
            });
            if !backed {
                return Err(anyhow!(
                    "golden eval query {} expected evidence ref is not backed by fixture corpus: {:?}",
                    query.id,
                    expected_ref
                ));
            }
        }
    }
    Ok(())
}

fn fixture_memory_for_validation(index: usize, memory: &GoldenMemory) -> crate::memory::Memory {
    crate::memory::Memory {
        id: index as i64 + 1,
        session_id: Some(format!("golden-eval-{}", index + 1)),
        project: memory.project.clone(),
        topic_key: memory.topic_key.clone(),
        title: memory.title.clone(),
        text: memory.content.clone(),
        memory_type: memory.memory_type.clone(),
        files: memory.files.clone(),
        created_at_epoch: memory.created_at_epoch.unwrap_or(0),
        updated_at_epoch: memory.created_at_epoch.unwrap_or(0),
        status: memory.status.clone(),
        branch: memory.branch.clone(),
        scope: memory.scope.clone(),
    }
}

pub(super) fn corpus_memory_matches_query_filter(
    memory: &crate::memory::Memory,
    query: &GoldenQuery,
) -> bool {
    if memory.status != "active" {
        return false;
    }
    if let Some(project) = query.project.as_deref() {
        if !crate::project_id::project_matches(Some(&memory.project), project) {
            return false;
        }
    }
    if let Some(branch) = query.branch.as_deref() {
        if memory.branch.as_deref() != Some(branch) {
            return false;
        }
    }
    if let Some(memory_type) = query.memory_type.as_deref() {
        if memory.memory_type != memory_type {
            return false;
        }
    }
    true
}

pub(in crate::eval) fn seed_fixture_corpus(
    conn: &Connection,
    corpus: &[GoldenMemory],
) -> Result<()> {
    for (index, memory) in corpus.iter().enumerate() {
        let id = crate::memory::insert_memory_full(
            conn,
            Some("golden-eval"),
            &memory.project,
            memory.topic_key.as_deref(),
            &memory.title,
            &memory.content,
            &memory.memory_type,
            memory.files.as_deref(),
            memory.branch.as_deref(),
            &memory.scope,
            memory.created_at_epoch,
        )
        .with_context(|| {
            format!(
                "seed golden eval corpus memory {}",
                corpus_memory_label(index, memory)
            )
        })?;
        if memory.status != "active" {
            conn.execute(
                "UPDATE memories SET status = ?1 WHERE id = ?2",
                params![memory.status, id],
            )
            .with_context(|| {
                format!(
                    "set golden eval corpus memory {} status",
                    corpus_memory_label(index, memory)
                )
            })?;
        }
        if memory.access_count.is_some() || memory.last_accessed_epoch.is_some() {
            conn.execute(
                "UPDATE memories
                 SET access_count = COALESCE(?1, access_count),
                     last_accessed_epoch = COALESCE(?2, last_accessed_epoch)
                 WHERE id = ?3",
                params![memory.access_count, memory.last_accessed_epoch, id],
            )
            .with_context(|| {
                format!(
                    "set golden eval corpus memory {} usage",
                    corpus_memory_label(index, memory)
                )
            })?;
        }
    }
    Ok(())
}

fn corpus_memory_label(index: usize, memory: &GoldenMemory) -> String {
    memory
        .topic_key
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| format!("#{}", index + 1))
}

pub(in crate::eval) fn evaluate_query(
    query: &GoldenQuery,
    results: &[crate::memory::Memory],
    k: usize,
    query_tokens: usize,
    retrieval_latency_ms: f64,
) -> QueryEvaluation {
    let expected_refs = query.expected_refs();
    if query.expects_abstention() {
        return QueryEvaluation {
            id: query.id.clone(),
            query: query.query.clone(),
            category: query.category.clone(),
            slice: query.slice_label().to_string(),
            status: if results.is_empty() {
                QueryStatus::Pass
            } else {
                QueryStatus::Fail
            },
            result_count: results.len(),
            retrieved_ids: retrieved_ids(results, k),
            expected_relevant_ids: expected_relevant_ids(&expected_refs),
            missing_relevant_ids: Vec::new(),
            missing_evidence_refs: Vec::new(),
            matched_refs: 0,
            expected_refs: expected_refs.len(),
            query_tokens,
            retrieval_latency_ms,
            metrics: None,
        };
    }

    if expected_refs.is_empty() {
        return QueryEvaluation {
            id: query.id.clone(),
            query: query.query.clone(),
            category: query.category.clone(),
            slice: query.slice_label().to_string(),
            status: QueryStatus::Skip,
            result_count: results.len(),
            retrieved_ids: retrieved_ids(results, k),
            expected_relevant_ids: Vec::new(),
            missing_relevant_ids: Vec::new(),
            missing_evidence_refs: Vec::new(),
            matched_refs: 0,
            expected_refs: 0,
            query_tokens,
            retrieval_latency_ms,
            metrics: None,
        };
    }

    let metrics = score_results(results, &expected_refs, k);
    let matched_ref_indexes = matched_ref_indexes(results, &expected_refs, k);
    let missing_evidence_refs = missing_evidence_refs(&expected_refs, &matched_ref_indexes);
    let missing_relevant_ids = missing_relevant_ids(&missing_evidence_refs);
    let matched_refs = matched_ref_indexes.len();
    let status = if metrics.hit_at_k > 0.0 {
        QueryStatus::Hit
    } else {
        QueryStatus::Miss
    };
    QueryEvaluation {
        id: query.id.clone(),
        query: query.query.clone(),
        category: query.category.clone(),
        slice: query.slice_label().to_string(),
        status,
        result_count: results.len(),
        retrieved_ids: retrieved_ids(results, k),
        expected_relevant_ids: expected_relevant_ids(&expected_refs),
        missing_relevant_ids,
        missing_evidence_refs,
        matched_refs,
        expected_refs: expected_refs.len(),
        query_tokens,
        retrieval_latency_ms,
        metrics: Some(metrics),
    }
}

pub(in crate::eval) fn estimate_query_tokens(query: &str) -> usize {
    query.len().div_ceil(4).max(1)
}

pub(in crate::eval) fn percentile(mut values: Vec<f64>, percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let rank = ((percentile / 100.0) * (values.len().saturating_sub(1) as f64)).ceil() as usize;
    values[rank.min(values.len() - 1)]
}

fn score_results(
    results: &[crate::memory::Memory],
    expected_refs: &[EvidenceRef],
    k: usize,
) -> QueryMetrics {
    let top_k = k.min(results.len());
    let top_rank_k = RANK_K.min(results.len());
    let relevance_at_k: Vec<bool> = results
        .iter()
        .take(top_k)
        .map(|memory| {
            expected_refs
                .iter()
                .any(|evidence_ref| evidence_ref.matches(memory))
        })
        .collect();
    let relevance_at_rank_k: Vec<bool> = results
        .iter()
        .take(top_rank_k)
        .map(|memory| {
            expected_refs
                .iter()
                .any(|evidence_ref| evidence_ref.matches(memory))
        })
        .collect();
    let matched_refs = matched_ref_indexes(results, expected_refs, k).len();
    let relevant_hits = relevance_at_k
        .iter()
        .filter(|is_relevant| **is_relevant)
        .count();
    let precision_denominator = top_k.max(1);

    QueryMetrics {
        hit_at_k: if relevant_hits > 0 { 1.0 } else { 0.0 },
        mrr_at_10: reciprocal_rank_from_relevance(&relevance_at_rank_k),
        precision_at_k: relevant_hits as f64 / precision_denominator as f64,
        recall_at_k: matched_refs as f64 / expected_refs.len() as f64,
        ndcg_at_10: ndcg_at_k(results, expected_refs, RANK_K),
        evidence_recall_at_k: matched_refs as f64 / expected_refs.len() as f64,
    }
}

fn matched_ref_indexes(
    results: &[crate::memory::Memory],
    expected_refs: &[EvidenceRef],
    k: usize,
) -> HashSet<usize> {
    let mut matched = HashSet::new();
    for memory in results.iter().take(k) {
        for (index, evidence_ref) in expected_refs.iter().enumerate() {
            if evidence_ref.matches(memory) {
                matched.insert(index);
            }
        }
    }
    matched
}

fn retrieved_ids(results: &[crate::memory::Memory], k: usize) -> Vec<i64> {
    results.iter().take(k).map(|memory| memory.id).collect()
}

fn expected_relevant_ids(expected_refs: &[EvidenceRef]) -> Vec<i64> {
    expected_refs
        .iter()
        .filter_map(|evidence_ref| evidence_ref.memory_id)
        .collect()
}

fn missing_evidence_refs(
    expected_refs: &[EvidenceRef],
    matched_ref_indexes: &HashSet<usize>,
) -> Vec<EvidenceRef> {
    expected_refs
        .iter()
        .enumerate()
        .filter_map(|(index, evidence_ref)| {
            (!matched_ref_indexes.contains(&index)).then_some(evidence_ref.clone())
        })
        .collect()
}

fn missing_relevant_ids(missing_evidence_refs: &[EvidenceRef]) -> Vec<i64> {
    missing_evidence_refs
        .iter()
        .filter_map(|evidence_ref| evidence_ref.memory_id)
        .collect()
}

fn reciprocal_rank_from_relevance(relevance: &[bool]) -> f64 {
    relevance
        .iter()
        .position(|is_relevant| *is_relevant)
        .map_or(0.0, |index| 1.0 / (index as f64 + 1.0))
}

fn ndcg_at_k(results: &[crate::memory::Memory], expected_refs: &[EvidenceRef], k: usize) -> f64 {
    if k == 0 || expected_refs.is_empty() {
        return 0.0;
    }

    let matches_by_rank: Vec<Vec<usize>> = results
        .iter()
        .take(k)
        .map(|memory| {
            expected_refs
                .iter()
                .enumerate()
                .filter_map(|(index, evidence_ref)| evidence_ref.matches(memory).then_some(index))
                .collect()
        })
        .collect();
    let dcg = best_dcg_for_matchable_ranks(&matches_by_rank);
    let ideal_hits = expected_refs.len().min(k);
    let idcg: f64 = (0..ideal_hits)
        .map(|index| 1.0 / (index as f64 + 2.0).log2())
        .sum();
    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}

fn best_dcg_for_matchable_ranks(matches_by_rank: &[Vec<usize>]) -> f64 {
    let mut best = 0.0;
    for mask in 1usize..(1usize << matches_by_rank.len()) {
        let dcg: f64 = (0..matches_by_rank.len())
            .filter(|rank| (mask & (1usize << rank)) != 0)
            .map(|rank| 1.0 / (rank as f64 + 2.0).log2())
            .sum();
        if dcg > best && can_assign_unique_refs(matches_by_rank, mask) {
            best = dcg;
        }
    }
    best
}

fn can_assign_unique_refs(matches_by_rank: &[Vec<usize>], mask: usize) -> bool {
    let mut selected_ranks: Vec<usize> = (0..matches_by_rank.len())
        .filter(|rank| (mask & (1usize << rank)) != 0)
        .collect();
    if selected_ranks
        .iter()
        .any(|rank| matches_by_rank[*rank].is_empty())
    {
        return false;
    }
    selected_ranks.sort_by_key(|rank| matches_by_rank[*rank].len());

    let mut assigned_refs = HashSet::new();
    assign_rank_ref(0, &selected_ranks, matches_by_rank, &mut assigned_refs)
}

fn assign_rank_ref(
    selected_index: usize,
    selected_ranks: &[usize],
    matches_by_rank: &[Vec<usize>],
    assigned_refs: &mut HashSet<usize>,
) -> bool {
    let Some(rank) = selected_ranks.get(selected_index).copied() else {
        return true;
    };

    for ref_index in &matches_by_rank[rank] {
        if assigned_refs.insert(*ref_index) {
            if assign_rank_ref(
                selected_index + 1,
                selected_ranks,
                matches_by_rank,
                assigned_refs,
            ) {
                return true;
            }
            assigned_refs.remove(ref_index);
        }
    }
    false
}
