pub(super) const SUMMARY_PROMPT: &str = include_str!("../../prompts/summary.txt");
pub(super) const COMPRESS_PROMPT: &str = include_str!("../../prompts/compress.txt");

pub(super) const SUMMARIZE_COOLDOWN_SECS: i64 = 300;
pub(super) const SUMMARIZE_LOCK_TIMEOUT_SECS: i64 = 180;
pub(super) const SUMMARIZE_STDIN_TIMEOUT_MS: u64 = 3000;

pub(super) const COMPRESS_THRESHOLD: i64 = 100;
pub(super) const KEEP_RECENT: i64 = 50;
pub(super) const COMPRESS_BATCH: i64 = 30;
