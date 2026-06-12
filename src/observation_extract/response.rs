use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;

use crate::db::models::OBSERVATION_TYPES;
use crate::memory::format::ParsedObservation;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ObservationExtractResponse {
    NoObservations,
    Observations(Vec<ParsedObservation>),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseEnvelope {
    observations: Option<Vec<JsonObservation>>,
    no_observations: Option<NoObservations>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoObservations {
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonObservation {
    #[serde(rename = "type")]
    obs_type: String,
    title: serde_json::Value,
    subtitle: serde_json::Value,
    narrative: serde_json::Value,
    facts: Vec<String>,
    concepts: Vec<String>,
    files_read: Vec<String>,
    files_modified: Vec<String>,
    confidence: f64,
}

pub(crate) fn parse_observation_extract_response(
    output: &str,
) -> Result<ObservationExtractResponse> {
    let envelope: ResponseEnvelope = serde_json::from_str(output.trim())
        .context("malformed observation_extract output: expected strict JSON object")?;
    match (envelope.observations, envelope.no_observations) {
        (Some(_), Some(_)) => bail!(
            "malformed observation_extract output: observations and no_observations are mutually exclusive"
        ),
        (None, None) => bail!(
            "malformed observation_extract output: missing observations or no_observations"
        ),
        (None, Some(no_observations)) => {
            ensure!(
                !no_observations.reason.trim().is_empty(),
                "malformed observation_extract output: no_observations.reason is required"
            );
            Ok(ObservationExtractResponse::NoObservations)
        }
        (Some(observations), None) => {
            ensure!(
                !observations.is_empty(),
                "malformed observation_extract output: observations must not be empty"
            );
            observations
                .into_iter()
                .enumerate()
                .map(|(index, observation)| validate_observation(index, observation))
                .collect::<Result<Vec<_>>>()
                .map(ObservationExtractResponse::Observations)
        }
    }
}

fn validate_observation(index: usize, observation: JsonObservation) -> Result<ParsedObservation> {
    let obs_type = observation.obs_type.trim().to_ascii_lowercase();
    ensure!(
        OBSERVATION_TYPES.contains(&obs_type.as_str()),
        "malformed observation_extract output: observation {index} has unsupported type `{}`",
        observation.obs_type
    );
    ensure!(
        observation.confidence.is_finite()
            && (0.0..=1.0).contains(&observation.confidence),
        "malformed observation_extract output: observation {index} confidence must be between 0.0 and 1.0"
    );

    let title = validate_observation_optional_text("title", index, observation.title)?;
    let subtitle = validate_observation_optional_text("subtitle", index, observation.subtitle)?;
    let narrative = validate_observation_optional_text("narrative", index, observation.narrative)?;
    let facts = validate_observation_text_list("facts", index, observation.facts)?;
    let concepts = validate_observation_text_list("concepts", index, observation.concepts)?;
    let files_read = validate_observation_text_list("files_read", index, observation.files_read)?;
    let files_modified =
        validate_observation_text_list("files_modified", index, observation.files_modified)?;

    ensure!(
        title.is_some() || narrative.is_some() || !facts.is_empty(),
        "malformed observation_extract output: observation {index} must include title, narrative, or facts"
    );

    Ok(ParsedObservation {
        obs_type,
        title,
        subtitle,
        facts,
        narrative,
        concepts,
        files_read,
        files_modified,
        confidence: Some(observation.confidence),
    })
}

fn validate_observation_optional_text(
    field: &str,
    index: usize,
    value: serde_json::Value,
) -> Result<Option<String>> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(value) => {
            let trimmed = value.trim();
            ensure!(
                !trimmed.is_empty(),
                "malformed observation_extract output: observation {index} {field} must not be blank"
            );
            Ok(Some(trimmed.to_string()))
        }
        _ => bail!(
            "malformed observation_extract output: observation {index} {field} must be a string or null"
        ),
    }
}

fn validate_observation_text_list(
    field: &str,
    index: usize,
    values: Vec<String>,
) -> Result<Vec<String>> {
    values
        .into_iter()
        .enumerate()
        .map(|(item_index, value)| {
            let trimmed = value.trim();
            ensure!(
                !trimmed.is_empty(),
                "malformed observation_extract output: observation {index} {field}[{item_index}] must not be blank"
            );
            Ok(trimmed.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_observations_json() -> Result<()> {
        let parsed = parse_observation_extract_response(
            r#"{
              "observations": [{
                "type": "decision",
                "title": "Use automatic capture",
                "subtitle": null,
                "narrative": "Automatic capture remains the primary path.",
                "facts": ["Manual save is supplemental."],
                "concepts": ["automatic capture"],
                "files_read": [],
                "files_modified": [],
                "confidence": 0.91
              }]
            }"#,
        )?;

        match parsed {
            ObservationExtractResponse::Observations(observations) => {
                assert_eq!(observations.len(), 1);
                assert_eq!(observations[0].obs_type, "decision");
                assert_eq!(observations[0].confidence, Some(0.91));
            }
            ObservationExtractResponse::NoObservations => panic!("expected observation"),
        }
        Ok(())
    }

    #[test]
    fn parses_explicit_no_observations_json() -> Result<()> {
        let parsed = parse_observation_extract_response(
            r#"{"no_observations":{"reason":"low signal command output"}}"#,
        )?;
        assert_eq!(parsed, ObservationExtractResponse::NoObservations);
        Ok(())
    }

    #[test]
    fn rejects_legacy_xml_output() {
        let err = parse_observation_extract_response(
            "<observation><type>discovery</type><narrative>legacy</narrative></observation>",
        )
        .expect_err("legacy xml must fail closed");
        assert!(err.to_string().contains("strict JSON object"));
    }

    #[test]
    fn rejects_unknown_fields() {
        let err = parse_observation_extract_response(
            r#"{
              "observations": [{
                "type": "decision",
                "title": "Known",
                "subtitle": null,
                "narrative": "Known",
                "facts": [],
                "concepts": [],
                "files_read": [],
                "files_modified": [],
                "confidence": 0.91,
                "extra": "nope"
              }]
            }"#,
        )
        .expect_err("unknown fields must fail closed");
        assert!(format!("{err:#}").contains("unknown field"));
    }

    #[test]
    fn rejects_missing_nullable_observation_fields() {
        let err = parse_observation_extract_response(
            r#"{
              "observations": [{
                "type": "decision",
                "subtitle": null,
                "narrative": "Known",
                "facts": [],
                "concepts": [],
                "files_read": [],
                "files_modified": [],
                "confidence": 0.91
              }]
            }"#,
        )
        .expect_err("missing title key must fail closed");
        assert!(format!("{err:#}").contains("missing field `title`"));
    }

    #[test]
    fn rejects_unsupported_types() {
        let err = parse_observation_extract_response(
            r#"{
              "observations": [{
                "type": "preference",
                "title": "No alias",
                "subtitle": null,
                "narrative": "Unsupported types must not default.",
                "facts": [],
                "concepts": [],
                "files_read": [],
                "files_modified": [],
                "confidence": 0.91
              }]
            }"#,
        )
        .expect_err("unsupported type must fail");
        assert!(err.to_string().contains("unsupported type"));
    }

    #[test]
    fn rejects_out_of_range_confidence() {
        let err = parse_observation_extract_response(
            r#"{
              "observations": [{
                "type": "decision",
                "title": "Bad confidence",
                "subtitle": null,
                "narrative": "Confidence is invalid.",
                "facts": [],
                "concepts": [],
                "files_read": [],
                "files_modified": [],
                "confidence": 1.7
              }]
            }"#,
        )
        .expect_err("out-of-range confidence must fail");
        assert!(err.to_string().contains("confidence must be between"));
    }
}
