mod config;
mod json_io;
mod paths;
mod runtime;
#[cfg(test)]
mod tests;

pub use runtime::{install, uninstall};
