use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub const ARTIFACT_VERSION: u32 = 2;
pub const LEGACY_ARTIFACT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledRulesArtifact {
    pub version: u32,
    pub compiled_at_epoch: i64,
    pub rules: Vec<CompiledRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledRule {
    pub rule_id: String,
    pub source_memory_id: i64,
    pub reinforcement_count: i64,
    pub action: RuleAction,
    pub override_state: RuleOverrideState,
    pub predicate: RulePredicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    Warn,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleOverrideState {
    pub disabled: bool,
    pub action_override: Option<RuleAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RulePredicate {
    CommandRegex { pattern: String, message: String },
    CommitTrailerForbidden { trailer: String, message: String },
    GitPushForceForbidden { message: String },
}

impl CompiledRulesArtifact {
    pub fn new(compiled_at_epoch: i64, rules: Vec<CompiledRule>) -> Self {
        Self {
            version: ARTIFACT_VERSION,
            compiled_at_epoch,
            rules,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if !matches!(self.version, LEGACY_ARTIFACT_VERSION | ARTIFACT_VERSION) {
            bail!(
                "unsupported compiled rule artifact version {}; expected {} or {}",
                self.version,
                LEGACY_ARTIFACT_VERSION,
                ARTIFACT_VERSION
            );
        }
        if self.compiled_at_epoch < 0 {
            bail!("compiled rule artifact has negative compiled_at_epoch");
        }
        for rule in &self.rules {
            rule.validate(self.version)?;
        }
        Ok(())
    }
}

impl CompiledRule {
    pub fn effective_action(&self) -> RuleAction {
        self.override_state.action_override.unwrap_or(self.action)
    }

    fn validate(&self, artifact_version: u32) -> Result<()> {
        if self.rule_id.trim().is_empty() {
            bail!("compiled rule has empty rule_id");
        }
        if self.source_memory_id <= 0 {
            bail!(
                "compiled rule {} has invalid source_memory_id",
                self.rule_id
            );
        }
        if self.reinforcement_count < 1 {
            bail!(
                "compiled rule {} has invalid reinforcement_count {}",
                self.rule_id,
                self.reinforcement_count
            );
        }
        self.predicate.validate(&self.rule_id, artifact_version)?;
        Ok(())
    }
}

impl RulePredicate {
    pub fn message(&self) -> &str {
        match self {
            RulePredicate::CommandRegex { message, .. }
            | RulePredicate::CommitTrailerForbidden { message, .. }
            | RulePredicate::GitPushForceForbidden { message } => message,
        }
    }

    fn validate(&self, rule_id: &str, artifact_version: u32) -> Result<()> {
        match self {
            RulePredicate::CommandRegex { pattern, message } => {
                if pattern.trim().is_empty() {
                    bail!("compiled rule {rule_id} has empty command_regex pattern");
                }
                let regex_error = if artifact_version == LEGACY_ARTIFACT_VERSION {
                    regex::Regex::new(pattern)
                        .err()
                        .map(|error| error.to_string())
                } else {
                    regex_lite::Regex::new(pattern)
                        .err()
                        .map(|error| error.to_string())
                };
                if let Some(error) = regex_error {
                    bail!("compiled rule {rule_id} has invalid command_regex pattern: {error}");
                }
                if message.trim().is_empty() {
                    bail!("compiled rule {rule_id} has empty command_regex message");
                }
            }
            RulePredicate::CommitTrailerForbidden { trailer, message } => {
                if trailer.trim().is_empty() {
                    bail!("compiled rule {rule_id} has empty forbidden trailer");
                }
                if message.trim().is_empty() {
                    bail!("compiled rule {rule_id} has empty forbidden trailer message");
                }
            }
            RulePredicate::GitPushForceForbidden { message } => {
                if artifact_version == LEGACY_ARTIFACT_VERSION {
                    bail!("compiled rule {rule_id} uses git_push_force_forbidden in a v1 artifact");
                }
                if message.trim().is_empty() {
                    bail!("compiled rule {rule_id} has empty forbidden force-push message");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::test_support::package_manager_rule;

    #[test]
    fn artifact_schema_round_trips_versioned_rules() -> Result<()> {
        let artifact =
            CompiledRulesArtifact::new(1234, vec![package_manager_rule(RuleAction::Warn)]);

        let text = serde_json::to_string_pretty(&artifact)?;
        let parsed: CompiledRulesArtifact = serde_json::from_str(&text)?;

        assert_eq!(parsed.version, ARTIFACT_VERSION);
        assert_eq!(parsed.compiled_at_epoch, 1234);
        assert_eq!(parsed.rules[0].rule_id, "pref-123-1");
        assert_eq!(parsed.rules[0].source_memory_id, 123);
        assert_eq!(parsed.rules[0].reinforcement_count, 3);
        assert_eq!(parsed.rules[0].action, RuleAction::Warn);
        assert_eq!(
            parsed.rules[0].predicate,
            RulePredicate::CommandRegex {
                pattern: r"(^|[ \t\r\n])npm[ \t\r\n]+(install|i|add)([ \t\r\n;&|)<>]|$)"
                    .to_string(),
                message: "Command violates a compiled package-manager preference".to_string()
            }
        );
        parsed.validate()?;
        Ok(())
    }

    #[test]
    fn artifact_rejects_unsupported_predicate_kind() {
        let text = r#"{
          "version": 1,
          "compiled_at_epoch": 1234,
          "rules": [{
            "rule_id": "pref-123-1",
            "source_memory_id": 123,
            "reinforcement_count": 3,
            "action": "warn",
            "override_state": {"disabled": false, "action_override": null},
            "predicate": {"kind": "javascript", "source": "return true"}
          }]
        }"#;

        let err = serde_json::from_str::<CompiledRulesArtifact>(text)
            .expect_err("unsupported predicate kind should fail closed at parse time");
        assert!(
            err.to_string().contains("unknown variant"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn artifact_validation_rejects_invalid_command_regex() {
        let mut rule = package_manager_rule(RuleAction::Warn);
        rule.predicate = RulePredicate::CommandRegex {
            pattern: "(".to_string(),
            message: "invalid regex fixture".to_string(),
        };
        let artifact = CompiledRulesArtifact::new(1234, vec![rule]);

        let error = artifact
            .validate()
            .expect_err("invalid command regex must fail artifact validation");

        assert!(error.to_string().contains("invalid command_regex pattern"));
    }

    #[test]
    fn legacy_artifact_keeps_unicode_regex_validation() -> Result<()> {
        let mut artifact =
            CompiledRulesArtifact::new(1234, vec![package_manager_rule(RuleAction::Warn)]);
        artifact.version = LEGACY_ARTIFACT_VERSION;
        artifact.rules[0].predicate = RulePredicate::CommandRegex {
            pattern: r"\p{Greek}+".to_string(),
            message: "legacy unicode fixture".to_string(),
        };

        artifact.validate()?;
        Ok(())
    }

    #[test]
    fn force_push_predicate_requires_v2_and_round_trips() -> Result<()> {
        let mut artifact =
            CompiledRulesArtifact::new(1234, vec![package_manager_rule(RuleAction::Warn)]);
        artifact.rules[0].predicate = RulePredicate::GitPushForceForbidden {
            message: "Do not force push".to_string(),
        };

        artifact.validate()?;
        let encoded = serde_json::to_string(&artifact)?;
        let parsed: CompiledRulesArtifact = serde_json::from_str(&encoded)?;
        assert_eq!(parsed, artifact);

        artifact.version = LEGACY_ARTIFACT_VERSION;
        let error = artifact
            .validate()
            .expect_err("v1 artifact must reject the v2-only predicate");
        assert!(error.to_string().contains("in a v1 artifact"));
        Ok(())
    }
}
