mod projection;
mod types;

pub use projection::{project_session, PROJECTION_VERSION};
pub use types::{ProjectionResult, SessionActivityKey, SessionTurn, TurnAction};

#[cfg(test)]
mod tests;
