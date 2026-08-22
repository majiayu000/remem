#[derive(Debug)]
pub struct LocalCopyError {
    message: String,
}

impl From<anyhow::Error> for LocalCopyError {
    fn from(error: anyhow::Error) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl std::fmt::Display for LocalCopyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for LocalCopyError {}

#[derive(Debug)]
pub struct SaveMemoryValidationError {
    message: String,
}

impl SaveMemoryValidationError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SaveMemoryValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SaveMemoryValidationError {}

#[derive(Debug)]
pub struct SaveMemoryIdempotencyConflictError {
    message: String,
}

impl SaveMemoryIdempotencyConflictError {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SaveMemoryIdempotencyConflictError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SaveMemoryIdempotencyConflictError {}
