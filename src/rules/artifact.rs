use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub const ARTIFACT_VERSION: u32 = 1;

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
        if self.version != ARTIFACT_VERSION {
            bail!(
                "unsupported compiled rule artifact version {}; expected {}",
                self.version,
                ARTIFACT_VERSION
            );
        }
        if self.compiled_at_epoch < 0 {
            bail!("compiled rule artifact has negative compiled_at_epoch");
        }
        for rule in &self.rules {
            rule.validate()?;
        }
        Ok(())
    }
}

impl CompiledRule {
    pub fn effective_action(&self) -> RuleAction {
        self.override_state.action_override.unwrap_or(self.action)
    }

    fn validate(&self) -> Result<()> {
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
        self.predicate.validate(&self.rule_id)?;
        Ok(())
    }
}

impl RulePredicate {
    pub fn message(&self) -> &str {
        match self {
            RulePredicate::CommandRegex { message, .. }
            | RulePredicate::CommitTrailerForbidden { message, .. } => message,
        }
    }

    fn validate(&self, rule_id: &str) -> Result<()> {
        match self {
            RulePredicate::CommandRegex { pattern, message } => {
                if pattern.trim().is_empty() {
                    bail!("compiled rule {rule_id} has empty command_regex pattern");
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
                pattern: r"(^|\s)npm\s+(install|i|add)\b".to_string(),
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
}
