pub(super) const DREAM_PROMPT: &str = include_str!("../../prompts/dream.txt");

/// 每个 project 每次 dream 处理的最大 cluster 数
pub(super) const DREAM_MAX_CLUSTERS: usize = 30;

/// cluster 内记忆数下限（少于这个不合并）
pub(super) const DREAM_MIN_CLUSTER_SIZE: usize = 2;

/// topic_key 前缀匹配长度
pub(super) const TOPIC_KEY_PREFIX_LEN: usize = 20;

/// dream job 最小触发间隔（秒）
#[allow(dead_code)]
pub(super) const DREAM_COOLDOWN_SECS: i64 = 3600;

/// 最近 N 秒内写入的记忆不参与合并（避免合并进行中的会话）
pub(super) const DREAM_RECENCY_GUARD_SECS: i64 = 3600;
