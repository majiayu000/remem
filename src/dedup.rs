mod access;
mod funnel;
mod hash;
#[cfg(test)]
mod tests;

pub use access::mark_duplicate_accessed;
pub use funnel::check_duplicate;
pub use hash::find_hash_duplicates;
