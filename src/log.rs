mod config;
mod timer;
mod write;
#[cfg(test)]
mod tests;

pub use timer::Timer;
pub use write::{debug, info, open_log_append, warn};
