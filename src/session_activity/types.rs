use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionActivityKey {
    pub source_root: String,
    pub project: String,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TurnAction {
    pub index: i64,
    pub kind: String,
    pub tool_name: Option<String>,
    pub summary: String,
    pub event_row_id: Option<i64>,
    pub files: Vec<String>,
    pub outcome: Option<String>,
    pub created_at_epoch: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionTurn {
    pub id: Option<i64>,
    pub turn_index: i64,
    pub user_message_id: i64,
    pub user_said: String,
    pub understanding_message_id: Option<i64>,
    pub understanding: Option<String>,
    pub understanding_source: Option<String>,
    pub result_message_id: Option<i64>,
    pub actions_summary: Option<String>,
    pub result_status: String,
    pub result_summary: Option<String>,
    pub started_at_epoch: i64,
    pub ended_at_epoch: Option<i64>,
    pub capture_health: String,
    pub actions: Vec<TurnAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectionResult {
    pub changed: bool,
    pub source_digest: String,
    pub turn_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RawSessionActivity {
    pub source_root: String,
    pub project: String,
    pub session_id: String,
    pub message_count: i64,
    pub user_message_count: i64,
    pub assistant_message_count: i64,
    pub first_epoch: i64,
    pub last_epoch: i64,
    pub projected_turn_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionActivityItem {
    pub source_root: String,
    pub project: String,
    pub session_id: String,
    #[serde(flatten)]
    pub turn: SessionTurn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActivityCount {
    pub key: String,
    pub count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionActivityStats {
    pub sessions: i64,
    pub turns: i64,
    pub actions: i64,
    pub result_status: Vec<ActivityCount>,
    pub capture_health: Vec<ActivityCount>,
    pub projects: Vec<ActivityCount>,
    pub tools: Vec<ActivityCount>,
}
