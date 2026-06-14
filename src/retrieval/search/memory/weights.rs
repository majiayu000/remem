const RRF_K: f64 = 60.0;
const MAX_VECTOR_DISTANCE: f32 = 0.51;
const FTS_WEIGHT: f64 = 2.5;
const VECTOR_WEIGHT: f64 = 3.0;
const ENTITY_WEIGHT: f64 = 1.25;
const TEMPORAL_WEIGHT: f64 = 1.0;
const FACT_WEIGHT: f64 = 1.4;
const LIKE_FALLBACK_WEIGHT: f64 = 0.25;
const MIN_EVIDENCE_CONFIDENCE: f64 = 0.62;

#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SearchWeights {
    pub fts: f64,
    pub vector: f64,
    pub entity: f64,
    pub temporal: f64,
    #[serde(default = "default_fact_weight")]
    pub fact: f64,
    pub like_fallback: f64,
    pub max_vector_distance: f32,
    pub rrf_k: f64,
    pub min_evidence_confidence: f64,
}

impl Default for SearchWeights {
    fn default() -> Self {
        Self {
            fts: FTS_WEIGHT,
            vector: VECTOR_WEIGHT,
            entity: ENTITY_WEIGHT,
            temporal: TEMPORAL_WEIGHT,
            fact: FACT_WEIGHT,
            like_fallback: LIKE_FALLBACK_WEIGHT,
            max_vector_distance: MAX_VECTOR_DISTANCE,
            rrf_k: RRF_K,
            min_evidence_confidence: MIN_EVIDENCE_CONFIDENCE,
        }
    }
}

fn default_fact_weight() -> f64 {
    FACT_WEIGHT
}
