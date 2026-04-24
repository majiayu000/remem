mod config;
#[cfg(test)]
mod tests;
mod timer;
mod write;

pub use timer::Timer;
pub use write::{debug, error, info, open_log_append, warn};
