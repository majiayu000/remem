mod config;
pub(crate) mod duplicates;
mod host;
mod hosts;
mod json_io;
mod paths;
mod runtime;
#[cfg(test)]
mod tests;

pub use host::InstallTarget;
pub use runtime::{install, uninstall};
