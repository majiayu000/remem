use anyhow::{bail, Result};

#[derive(Debug, Clone, Default)]
pub(super) struct BackupAcknowledgement {
    pub(super) pattern_id: Option<String>,
    pub(super) pattern_version: Option<i64>,
    pub(super) acknowledged_at_epoch: Option<i64>,
}

impl BackupAcknowledgement {
    pub(super) fn validate_metadata(&self) -> Result<bool> {
        match (
            &self.pattern_id,
            self.pattern_version,
            self.acknowledged_at_epoch,
        ) {
            (None, None, None) => Ok(false),
            (Some(pattern_id), Some(version), Some(acknowledged_at_epoch))
                if !pattern_id.trim().is_empty() && version > 0 && acknowledged_at_epoch > 0 =>
            {
                Ok(true)
            }
            _ => bail!("backup import acknowledgement metadata is incomplete"),
        }
    }

    pub(super) fn validate_for_payload(&self, title: &str, content: &str) -> Result<bool> {
        let present = self.validate_metadata()?;
        let matched =
            crate::memory::poisoning::scan_instruction_pattern(&format!("{title}\n{content}"));
        match (matched, present) {
            (Some(matched), true)
                if self.pattern_id.as_deref() == Some(matched.pattern_id)
                    && self.pattern_version == Some(matched.pattern_set_version) =>
            {
                Ok(true)
            }
            (Some(matched), true) => bail!(
                "backup import acknowledgement does not match instruction-pattern {}@{}",
                matched.pattern_id,
                matched.pattern_set_version
            ),
            (Some(matched), false) => bail!(
                "backup import payload matched instruction-pattern {}@{}",
                matched.pattern_id,
                matched.pattern_set_version
            ),
            (None, true) => Ok(true),
            (None, false) => Ok(false),
        }
    }
}
