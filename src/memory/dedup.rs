mod access;
mod funnel;
mod hash;
#[cfg(test)]
mod tests;

pub use access::mark_duplicate_accessed;
pub use funnel::check_duplicate;
pub(crate) use funnel::check_duplicate_texts;
pub(crate) use hash::canonical_observation_text;
pub use hash::find_hash_duplicates;
