use anyhow::{anyhow, Result};

use crate::db;

#[derive(serde::Deserialize)]
struct GoldenDataset {
    queries: Vec<GoldenQuery>,
}

#[derive(serde::Deserialize)]
struct GoldenQuery {
    id: String,
    query: String,
    category: String,
    project: Option<String>,
    relevant_ids: Vec<i64>,
}

pub(in crate::cli) fn run_eval_local() -> Result<()> {
    let conn = db::open_db()?;
    let report = crate::eval_local::run_eval(&conn)?;
    print!("{}", report);
    Ok(())
}

pub(in crate::cli) fn run_eval(dataset_path: &str, k: usize) -> Result<()> {
    let content = std::fs::read_to_string(dataset_path)
        .map_err(|error| anyhow!("cannot read {}: {}", dataset_path, error))?;
    let dataset: GoldenDataset = serde_json::from_str(&content)?;
    let conn = db::open_db()?;

    let mut total_rr = 0.0;
    let mut total_p = 0.0;
    let mut total_r = 0.0;
    let mut total_hit = 0.0;
    let mut evaluated = 0usize;

    println!("remem eval — {} queries, k={}\n", dataset.queries.len(), k);

    for query in &dataset.queries {
        let results = crate::search::search(
            &conn,
            Some(&query.query),
            query.project.as_deref(),
            None,
            k as i64,
            0,
            false,
        )?;
        let result_ids: Vec<i64> = results.iter().map(|memory| memory.id).collect();

        let rr = crate::eval_metrics::reciprocal_rank(&result_ids, &query.relevant_ids);
        let precision = crate::eval_metrics::precision_at_k(&result_ids, &query.relevant_ids, k);
        let recall = crate::eval_metrics::recall_at_k(&result_ids, &query.relevant_ids, k);
        let hit = crate::eval_metrics::hit_at_k(&result_ids, &query.relevant_ids, k);
        let status = eval_status(query, &results, hit);

        println!(
            "  [{}] {:>4} | P@{}={:.2} R@{}={:.2} RR={:.2} | {} | {}",
            query.id, status, k, precision, k, recall, rr, query.category, query.query
        );

        if !query.relevant_ids.is_empty() {
            total_rr += rr;
            total_p += precision;
            total_r += recall;
            total_hit += hit;
            evaluated += 1;
        }
    }

    print_aggregate_metrics(k, evaluated, total_rr, total_p, total_r, total_hit);
    Ok(())
}

fn eval_status(query: &GoldenQuery, results: &[crate::memory::Memory], hit: f64) -> &'static str {
    if query.relevant_ids.is_empty() {
        if results.is_empty() {
            "PASS"
        } else {
            "---"
        }
    } else if hit > 0.0 {
        "HIT"
    } else {
        "MISS"
    }
}

fn print_aggregate_metrics(
    k: usize,
    evaluated: usize,
    total_rr: f64,
    total_p: f64,
    total_r: f64,
    total_hit: f64,
) {
    if evaluated == 0 {
        return;
    }

    let n = evaluated as f64;
    println!(
        "\n--- Aggregate ({} queries with ground truth) ---",
        evaluated
    );
    println!("  MRR:          {:.3}", total_rr / n);
    println!("  Precision@{}:  {:.3}", k, total_p / n);
    println!("  Recall@{}:     {:.3}", k, total_r / n);
    println!("  Hit Rate@{}:   {:.3}", k, total_hit / n);
}
