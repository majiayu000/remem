use anyhow::{bail, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SupplementalSaveReceipt {
    Saved { claim_id: i64 },
    Disabled,
    Failed { error: String },
}

impl SupplementalSaveReceipt {
    pub(crate) fn saved(claim_id: i64) -> Result<Self> {
        if claim_id <= 0 {
            bail!("supplemental save receipt claim id must be positive");
        }
        Ok(Self::Saved { claim_id })
    }

    pub(crate) fn failed(error: impl Into<String>) -> Result<Self> {
        let error = error.into();
        if error.trim().is_empty() || error.contains('\0') {
            bail!("supplemental save receipt failure must be nonblank and contain no NUL");
        }
        Ok(Self::Failed { error })
    }

    pub(crate) fn status(&self) -> &'static str {
        match self {
            Self::Saved { .. } => "saved",
            Self::Disabled => "disabled",
            Self::Failed { .. } => "failed",
        }
    }

    pub(crate) fn claim_id(&self) -> Option<i64> {
        match self {
            Self::Saved { claim_id } => Some(*claim_id),
            Self::Disabled | Self::Failed { .. } => None,
        }
    }

    pub(crate) fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error } => Some(error),
            Self::Saved { .. } | Self::Disabled => None,
        }
    }

    pub(super) fn from_columns(
        status: Option<String>,
        claim_id: Option<i64>,
        error: Option<String>,
    ) -> Result<Option<Self>> {
        match (status.as_deref(), claim_id, error) {
            (None, None, None) => Ok(None),
            (Some("saved"), Some(claim_id), None) => Self::saved(claim_id).map(Some),
            (Some("disabled"), None, None) => Ok(Some(Self::Disabled)),
            (Some("failed"), None, Some(error)) => Self::failed(error).map(Some),
            _ => bail!("stored supplemental save receipt has an invalid field combination"),
        }
    }
}
