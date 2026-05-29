use rmcp::model::{Content, IntoContents};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum McpErrorCode {
    InvalidRequest,
    NotFound,
    DbOpenFailed,
    DbQueryFailed,
    SerializationFailed,
    UnsupportedSource,
}

impl McpErrorCode {
    pub(super) fn wire_code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::NotFound => "not_found",
            Self::DbOpenFailed => "db_open_failed",
            Self::DbQueryFailed => "db_query_failed",
            Self::SerializationFailed => "serialization_failed",
            Self::UnsupportedSource => "unsupported_source",
        }
    }

    fn retryable(self) -> bool {
        matches!(self, Self::DbOpenFailed | Self::DbQueryFailed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct McpToolError {
    tool: &'static str,
    code: McpErrorCode,
    message: String,
    retryable: bool,
}

impl McpToolError {
    pub(super) fn new(tool: &'static str, code: McpErrorCode, message: impl Into<String>) -> Self {
        Self {
            tool,
            code,
            message: message.into(),
            retryable: code.retryable(),
        }
    }

    pub(super) fn db_open(tool: &'static str, err: impl std::fmt::Display) -> Self {
        Self::new(
            tool,
            McpErrorCode::DbOpenFailed,
            format!("DB open failed: {err}"),
        )
    }

    pub(super) fn db_query(tool: &'static str, err: impl std::fmt::Display) -> Self {
        Self::new(tool, McpErrorCode::DbQueryFailed, err.to_string())
    }

    pub(super) fn serialization(tool: &'static str, err: impl std::fmt::Display) -> Self {
        Self::new(
            tool,
            McpErrorCode::SerializationFailed,
            format!("serialization failed: {err}"),
        )
    }

    pub(super) fn invalid_request(tool: &'static str, message: impl Into<String>) -> Self {
        Self::new(tool, McpErrorCode::InvalidRequest, message)
    }

    pub(super) fn not_found(tool: &'static str, message: impl Into<String>) -> Self {
        Self::new(tool, McpErrorCode::NotFound, message)
    }

    pub(super) fn unsupported_source(tool: &'static str, message: impl Into<String>) -> Self {
        Self::new(tool, McpErrorCode::UnsupportedSource, message)
    }

    #[cfg(test)]
    pub(super) fn code(&self) -> McpErrorCode {
        self.code
    }

    pub(super) fn envelope(&self) -> Value {
        json!({
            "error": {
                "code": self.code.wire_code(),
                "message": self.message,
                "retryable": self.retryable,
                "tool": self.tool,
            }
        })
    }

    pub(super) fn to_json_string(&self) -> String {
        serde_json::to_string(&self.envelope()).unwrap_or_else(|_| {
            format!(
                r#"{{"error":{{"code":"{}","message":"{}","retryable":{},"tool":"{}"}}}}"#,
                self.code.wire_code(),
                "failed to serialize MCP error envelope",
                self.retryable,
                self.tool
            )
        })
    }
}

impl std::fmt::Display for McpToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_json_string())
    }
}

impl IntoContents for McpToolError {
    fn into_contents(self) -> Vec<Content> {
        vec![Content::text(self.to_json_string())]
    }
}

pub(super) type McpToolResult<T> = Result<T, McpToolError>;

pub(super) fn to_json_pretty<T: Serialize + ?Sized>(
    tool: &'static str,
    value: &T,
) -> McpToolResult<String> {
    serde_json::to_string_pretty(value).map_err(|err| McpToolError::serialization(tool, err))
}

pub(super) fn to_json_string<T: Serialize + ?Sized>(
    tool: &'static str,
    value: &T,
) -> McpToolResult<String> {
    serde_json::to_string(value).map_err(|err| McpToolError::serialization(tool, err))
}

pub(super) fn to_json_value<T: Serialize + ?Sized>(
    tool: &'static str,
    value: &T,
) -> McpToolResult<Value> {
    serde_json::to_value(value).map_err(|err| McpToolError::serialization(tool, err))
}
