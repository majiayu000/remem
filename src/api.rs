mod auth;
// SP880-T1 lands shared primitives before the T2-T4 handlers consume them.
#[allow(dead_code)]
pub(crate) mod cursor;
mod handlers;
mod helpers;
#[allow(dead_code)]
pub(crate) mod mutation;
mod server;
#[cfg(test)]
mod tests;
mod types;

pub use auth::{ensure_api_token, load_api_token};
pub use server::{build_router, run_api_server};
pub use types::DbState;
