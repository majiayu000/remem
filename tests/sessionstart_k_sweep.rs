use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const REQUIRED_KS: [u64; 4] = [1, 3, 5, 10];

fn smallest_eligible_k(report: &Value) -> Option<u64> {
    let tolerance = report["selection_rule"]["tolerance"].as_f64()?;
    let expected_best = report["populated_slice_best_hit_at_k"].as_object()?;
    if expected_best.is_empty() {
        return None;
    }
    let arms = report["arms"].as_array()?;
    let by_k = arms
        .iter()
        .map(|arm| Some((arm["k"].as_u64()?, arm)))
        .collect::<Option<BTreeMap<_, _>>>()?;
    if by_k.keys().copied().collect::<BTreeSet<_>>()
        != REQUIRED_KS.into_iter().collect::<BTreeSet<_>>()
    {
        return None;
    }

    let mut computed_best = BTreeMap::new();
    for slice in expected_best.keys() {
        let best = REQUIRED_KS
            .iter()
            .map(|k| by_k[k]["slice_hit_at_k"][slice].as_f64())
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .reduce(f64::max)?;
        computed_best.insert(slice.as_str(), best);
        if (best - expected_best[slice].as_f64()?).abs() > f64::EPSILON {
            return None;
        }
    }

    REQUIRED_KS.into_iter().find(|k| {
        let arm = by_k[k];
        arm["gate"]["passed"].as_bool() == Some(true)
            && computed_best.iter().all(|(slice, best)| {
                arm["slice_hit_at_k"][slice]
                    .as_f64()
                    .is_some_and(|score| score + tolerance >= *best)
            })
    })
}

fn synthetic_report(ks: &[u64]) -> Value {
    json!({
        "selection_rule": {"tolerance": 0.01},
        "populated_slice_best_hit_at_k": {"slice_a": 1.0, "slice_b": 0.5},
        "arms": ks.iter().map(|k| json!({
            "k": k,
            "slice_hit_at_k": {"slice_a": 1.0, "slice_b": 0.5},
            "gate": {"passed": true}
        })).collect::<Vec<_>>()
    })
}

#[test]
fn committed_report_is_complete_and_recommends_smallest_tied_arm() {
    let report: Value =
        serde_json::from_str(include_str!("../eval/sessionstart-k-sweep/report.json"))
            .expect("committed SessionStart k-sweep report must be valid JSON");

    assert_eq!(smallest_eligible_k(&report), Some(1));
    assert_eq!(report["decision"]["default_k"], 1);
}

#[test]
fn recommendation_chooses_smallest_complete_tied_arm() {
    assert_eq!(
        smallest_eligible_k(&synthetic_report(&REQUIRED_KS)),
        Some(1)
    );
}

#[test]
fn recommendation_rejects_missing_arm() {
    assert_eq!(smallest_eligible_k(&synthetic_report(&[1, 3, 5])), None);
}

#[test]
fn recommendation_rejects_missing_populated_slice_data() {
    let mut report = synthetic_report(&REQUIRED_KS);
    report["arms"][2]["slice_hit_at_k"]
        .as_object_mut()
        .expect("slice object")
        .remove("slice_b");

    assert_eq!(smallest_eligible_k(&report), None);
}
