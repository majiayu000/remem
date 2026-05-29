mod index;
mod paths;
mod render;
mod runtime;
#[cfg(test)]
mod tests;

pub(crate) use render::REMEM_FILE;
pub use runtime::sync_to_claude_memory;
pub(crate) use runtime::{
    native_memory_max_bytes, native_memory_sync_disabled, DISABLE_NATIVE_MEMORY_SYNC_ENV,
    NATIVE_MEMORY_MAX_BYTES_ENV,
};
