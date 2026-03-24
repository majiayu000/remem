use std::collections::HashMap;
use std::sync::LazyLock;

static SYNONYMS: LazyLock<HashMap<&str, &[&str]>> = LazyLock::new(|| {
    let mut m: HashMap<&str, &[&str]> = HashMap::new();
    // English → Chinese + related terms
    m.insert("encrypt", &["加密", "encryption", "sqlcipher"]);
    m.insert("encryption", &["加密", "encrypt", "sqlcipher"]);
    m.insert("sqlcipher", &["加密", "encryption", "encrypt"]);
    m.insert("database", &["数据库", "sqlite", "db"]);
    m.insert("db", &["数据库", "database", "sqlite"]);
    m.insert("api", &["接口", "endpoint", "端点"]);
    m.insert("rest", &["http", "接口", "api"]);
    m.insert("hook", &["钩子", "hooks"]);
    m.insert("trait", &["特征", "接口", "adapter"]);
    m.insert("adapter", &["适配器", "trait"]);
    m.insert("benchmark", &["基准测试", "性能测试", "eval"]);
    m.insert("compression", &["压缩", "compaction", "compress"]);
    m.insert("timeout", &["超时", "timed out"]);
    m.insert("worker", &["后台任务", "工作线程"]);
    m.insert("memory", &["记忆", "memories"]);
    m.insert("search", &["搜索", "检索", "fts"]);
    m.insert("fts", &["搜索", "全文搜索", "fts5"]);
    m.insert("tokenizer", &["分词器", "trigram"]);
    m.insert("prompt", &["提示词"]);
    m.insert("publish", &["发布", "发帖"]);
    m.insert("posting", &["发帖", "发布"]);
    m.insert("deploy", &["部署", "上线"]);
    m.insert("config", &["配置", "configuration", "设置"]);
    m.insert("port", &["端口"]);
    m.insert("session", &["会话"]);
    m.insert("scope", &["作用域", "范围"]);
    m.insert("global", &["全局", "跨项目"]);
    // Chinese → English + related terms
    m.insert("加密", &["encrypt", "encryption", "sqlcipher"]);
    m.insert("数据库", &["database", "sqlite", "db"]);
    m.insert("接口", &["api", "interface", "trait"]);
    m.insert("搜索", &["search", "fts", "检索"]);
    m.insert("检索", &["search", "fts", "搜索"]);
    m.insert("竞品", &["competitive", "comparison", "对比"]);
    m.insert("对比", &["comparison", "竞品", "vs"]);
    m.insert("优化", &["optimization", "optimize", "性能"]);
    m.insert("质量", &["quality"]);
    m.insert("记忆", &["memory", "memories"]);
    m.insert("共享", &["sharing", "global", "跨项目"]);
    m.insert("偏好", &["preference", "设置"]);
    m.insert("发帖", &["posting", "publish", "发布"]);
    m.insert("发布", &["publish", "release", "deploy"]);
    m.insert("挂起", &["hang", "卡住", "stuck"]);
    m.insert("超时", &["timeout", "timed out"]);
    m.insert("压缩", &["compression", "compress"]);
    m.insert("配置", &["config", "configuration", "设置"]);
    m.insert("端口", &["port"]);
    m.insert("架构", &["architecture", "design"]);
    m.insert("分词", &["tokenizer", "tokenize"]);
    m.insert("规则", &["rules", "rule"]);
    m.insert("部署", &["deploy", "deployment"]);
    m
});

/// Expand query tokens with synonyms. Returns deduplicated expanded list.
pub fn expand_query(raw: &str) -> Vec<String> {
    let mut expanded = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for token in raw.split_whitespace() {
        if seen.insert(token.to_lowercase()) {
            expanded.push(token.to_string());
        }
        let lower = token.to_lowercase();
        if let Some(syns) = SYNONYMS.get(lower.as_str()) {
            for syn in *syns {
                if seen.insert(syn.to_lowercase()) {
                    expanded.push(syn.to_string());
                }
            }
        }
    }
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_english_to_chinese() {
        let expanded = expand_query("encrypt");
        assert!(expanded.contains(&"加密".to_string()));
        assert!(expanded.contains(&"sqlcipher".to_string()));
    }

    #[test]
    fn expand_chinese_to_english() {
        let expanded = expand_query("数据库");
        assert!(expanded.contains(&"database".to_string()));
        assert!(expanded.contains(&"sqlite".to_string()));
    }

    #[test]
    fn expand_multi_token() {
        let expanded = expand_query("数据库 加密");
        assert!(expanded.contains(&"database".to_string()));
        assert!(expanded.contains(&"encrypt".to_string()));
    }

    #[test]
    fn no_duplicates() {
        let expanded = expand_query("encrypt encryption");
        let count = expanded.iter().filter(|t| t.to_lowercase() == "encrypt").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn unknown_word_passes_through() {
        let expanded = expand_query("foobar");
        assert_eq!(expanded, vec!["foobar"]);
    }
}
