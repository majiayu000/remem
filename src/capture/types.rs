use crate::identity::CaptureIdentity;

/// Normalized hook event ready for `captured_events` insertion.
///
/// Adapters (Claude Code / Codex CLI) translate raw hook payloads into this
/// shape; the capture entry point sees only `NormalizedEvent` so storage
/// stays host-agnostic.
#[derive(Debug, Clone)]
pub struct NormalizedEvent {
    pub identity: CaptureIdentity,
    /// One of: `user_message` | `assistant_message` | `tool_call` |
    /// `tool_result` | `file_edit` | `session_stop`.
    pub event_type: String,
    /// `user` | `assistant` | `tool` | `system` (free-form for now).
    pub role: Option<String>,
    pub tool_name: Option<String>,
    /// Adapter-supplied content. `insert_captured_event` may downgrade this
    /// to a truncated form when it exceeds the 16 KiB direct-storage limit.
    pub content_text: Option<String>,
    pub token_estimate: i64,
    /// Initial retention class assigned by the adapter; `insert` may flip it
    /// to `"raw_compact"` when content overflows the direct-storage budget.
    pub retention_class: String,
    pub created_at_epoch: i64,
}
