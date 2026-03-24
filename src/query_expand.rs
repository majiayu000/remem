use std::collections::{HashMap, HashSet};
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
    m.insert("hook", &["钩子", "hooks", "机制"]);
    m.insert("hooks", &["钩子", "hook", "机制"]);
    m.insert("trait", &["特征", "接口", "adapter"]);
    m.insert("adapter", &["适配器", "trait", "ToolAdapter"]);
    m.insert("benchmark", &["基准测试", "性能测试", "eval", "bench"]);
    m.insert("compression", &["压缩", "compaction", "compress"]);
    m.insert("timeout", &["超时", "timed out"]);
    m.insert("worker", &["后台任务", "工作线程"]);
    m.insert("memory", &["记忆", "memories", "memo"]);
    m.insert("search", &["搜索", "检索", "fts", "查询"]);
    m.insert("fts", &["搜索", "全文搜索", "fts5"]);
    m.insert("fts5", &["搜索", "全文搜索", "fts", "trigram"]);
    m.insert("tokenizer", &["分词器", "trigram", "分词"]);
    m.insert("prompt", &["提示词"]);
    m.insert("publish", &["发布", "发帖"]);
    m.insert("posting", &["发帖", "发布"]);
    m.insert("deploy", &["部署", "上线"]);
    m.insert("config", &["配置", "configuration", "设置"]);
    m.insert("port", &["端口"]);
    m.insert("session", &["会话"]);
    m.insert("scope", &["作用域", "范围"]);
    m.insert("global", &["全局", "跨项目"]);
    m.insert("summary", &["摘要", "总结", "格式"]);
    m.insert("format", &["格式", "结构"]);
    m.insert("mcp", &["server", "工具", "protocol"]);
    m.insert("server", &["服务器", "服务端"]);
    m.insert("twitter", &["X", "推特", "发帖"]);
    m.insert("promote", &["提升", "升级"]);
    m.insert("auto", &["自动", "自动化"]);
    m.insert("performance", &["性能", "优化", "benchmark"]);
    m.insert("video", &["视频"]);
    m.insert("quality", &["质量"]);
    m.insert("cost", &["成本"]);
    m.insert("context", &["上下文", "语境"]);
    // Chinese → English + related terms
    m.insert("加密", &["encrypt", "encryption", "sqlcipher"]);
    m.insert("数据库", &["database", "sqlite", "db"]);
    m.insert("接口", &["api", "interface", "trait"]);
    m.insert("搜索", &["search", "fts", "检索", "查询"]);
    m.insert("检索", &["search", "fts", "搜索"]);
    m.insert("竞品", &["competitive", "comparison", "对比"]);
    m.insert("对比", &["comparison", "竞品", "vs"]);
    m.insert("优化", &["optimization", "optimize", "性能"]);
    m.insert("质量", &["quality", "品质"]);
    m.insert("记忆", &["memory", "memories", "memo"]);
    m.insert("共享", &["sharing", "global", "跨项目"]);
    m.insert("偏好", &["preference", "设置"]);
    m.insert("发帖", &["posting", "publish", "发布", "twitter"]);
    m.insert("发布", &["publish", "release", "deploy"]);
    m.insert("挂起", &["hang", "卡住", "stuck"]);
    m.insert("超时", &["timeout", "timed out"]);
    m.insert("压缩", &["compression", "compress"]);
    m.insert("配置", &["config", "configuration", "设置"]);
    m.insert("端口", &["port"]);
    m.insert("架构", &["architecture", "design", "设计"]);
    m.insert("分词", &["tokenizer", "tokenize", "trigram"]);
    m.insert("规则", &["rules", "rule"]);
    m.insert("部署", &["deploy", "deployment"]);
    m.insert("失败", &["failure", "failed", "error", "错误"]);
    m.insert("任务", &["task", "job"]);
    m.insert("机制", &["mechanism", "hook", "hooks", "系统"]);
    m.insert("提升", &["promote", "升级", "improve"]);
    m.insert("自动", &["auto", "automatic", "自动化"]);
    m.insert("视频", &["video"]);
    m.insert("抖音", &["douyin", "tiktok"]);
    m.insert("成本", &["cost"]);
    m.insert("性能", &["performance", "benchmark", "优化"]);
    m.insert("格式", &["format", "structure", "结构"]);
    m.insert("摘要", &["summary", "总结"]);
    m.insert("总结", &["summary", "摘要"]);
    m.insert("会话", &["session"]);
    m.insert("实现", &["implementation", "implement"]);
    m.insert("服务", &["server", "service"]);
    m.insert("跨项目", &["cross-project", "global", "共享"]);
    m.insert("上下文", &["context"]);
    m.insert("钩子", &["hook", "hooks"]);
    m.insert("适配器", &["adapter", "trait"]);
    m
});

/// Check if a character is CJK (Chinese/Japanese/Korean).
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}' |   // CJK Unified Ideographs
        '\u{3400}'..='\u{4DBF}' |   // CJK Extension A
        '\u{F900}'..='\u{FAFF}'     // CJK Compatibility
    )
}

/// Split a string into CJK and non-CJK segments.
/// "数据库加密test" -> ["数据库加密", "test"]
/// "Claude Code hook 机制" -> ["Claude", "Code", "hook", "机制"]
fn tokenize_mixed(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for part in raw.split_whitespace() {
        let chars: Vec<char> = part.chars().collect();
        if chars.is_empty() {
            continue;
        }
        let mut i = 0;
        while i < chars.len() {
            if is_cjk(chars[i]) {
                // Collect consecutive CJK chars
                let start = i;
                while i < chars.len() && is_cjk(chars[i]) {
                    i += 1;
                }
                tokens.push(chars[start..i].iter().collect());
            } else {
                // Collect consecutive non-CJK chars
                let start = i;
                while i < chars.len() && !is_cjk(chars[i]) {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    tokens.push(trimmed.to_string());
                }
            }
        }
    }
    tokens
}

/// Dictionary-based maximum forward matching for CJK text.
/// Tries to split CJK text into known synonym keys.
/// "数据库加密" -> ["数据库", "加密"]
/// "跨项目记忆共享" -> ["跨项目", "记忆", "共享"]
fn segment_cjk(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut segments = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        let mut best_len = 0;
        // Try lengths 4, 3, 2 (max synonym key length down to 2)
        for len in (2..=4).rev() {
            if i + len <= chars.len() {
                let candidate: String = chars[i..i + len].iter().collect();
                if SYNONYMS.contains_key(candidate.as_str()) {
                    best_len = len;
                    break;
                }
            }
        }
        if best_len > 0 {
            segments.push(chars[i..i + best_len].iter().collect());
            i += best_len;
        } else {
            // Single char - skip or include as-is
            // For single CJK chars, include them (they might match via LIKE)
            segments.push(chars[i..i + 1].iter().collect());
            i += 1;
        }
    }
    segments
}

/// Expand query tokens with synonyms. Returns deduplicated expanded list.
/// Handles CJK text by segmenting first, then expanding each segment.
pub fn expand_query(raw: &str) -> Vec<String> {
    let mut expanded = Vec::new();
    let mut seen = HashSet::new();

    let mixed_tokens = tokenize_mixed(raw);

    for token in &mixed_tokens {
        let chars: Vec<char> = token.chars().collect();
        let all_cjk = !chars.is_empty() && chars.iter().all(|c| is_cjk(*c));

        if all_cjk && chars.len() > 1 {
            // Try dictionary segmentation first
            let segments = segment_cjk(token);
            let any_multi = segments.iter().any(|s| s.chars().count() > 1);

            if any_multi {
                // Successfully segmented — expand each segment
                for seg in &segments {
                    add_with_synonyms(seg, &mut expanded, &mut seen);
                }
                // Also keep the original unsegmented form for exact matching
                if seen.insert(token.to_lowercase()) {
                    expanded.push(token.to_string());
                }
            } else {
                // No multi-char segments found, keep original
                add_with_synonyms(token, &mut expanded, &mut seen);
            }
        } else {
            add_with_synonyms(token, &mut expanded, &mut seen);
        }
    }
    expanded
}

fn add_with_synonyms(token: &str, expanded: &mut Vec<String>, seen: &mut HashSet<String>) {
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
        let count = expanded
            .iter()
            .filter(|t| t.to_lowercase() == "encrypt")
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn unknown_word_passes_through() {
        let expanded = expand_query("foobar");
        assert_eq!(expanded, vec!["foobar"]);
    }

    #[test]
    fn cjk_segmentation_database_encrypt() {
        let expanded = expand_query("数据库加密");
        assert!(
            expanded.contains(&"数据库".to_string()),
            "should segment 数据库: {:?}",
            expanded
        );
        assert!(
            expanded.contains(&"加密".to_string()),
            "should segment 加密: {:?}",
            expanded
        );
        assert!(
            expanded.contains(&"database".to_string()),
            "should expand 数据库→database: {:?}",
            expanded
        );
        assert!(
            expanded.contains(&"encrypt".to_string()),
            "should expand 加密→encrypt: {:?}",
            expanded
        );
    }

    #[test]
    fn cjk_segmentation_cross_project_sharing() {
        let expanded = expand_query("跨项目记忆共享");
        assert!(
            expanded.contains(&"跨项目".to_string()),
            "should segment 跨项目: {:?}",
            expanded
        );
        assert!(
            expanded.contains(&"记忆".to_string()),
            "should segment 记忆: {:?}",
            expanded
        );
        assert!(
            expanded.contains(&"共享".to_string()),
            "should segment 共享: {:?}",
            expanded
        );
    }

    #[test]
    fn cjk_segmentation_memory_quality() {
        let expanded = expand_query("记忆质量");
        assert!(
            expanded.contains(&"记忆".to_string()),
            "should segment 记忆: {:?}",
            expanded
        );
        assert!(
            expanded.contains(&"质量".to_string()),
            "should segment 质量: {:?}",
            expanded
        );
        assert!(
            expanded.contains(&"memory".to_string()),
            "should expand to memory: {:?}",
            expanded
        );
        assert!(
            expanded.contains(&"quality".to_string()),
            "should expand to quality: {:?}",
            expanded
        );
    }

    #[test]
    fn mixed_cjk_and_ascii() {
        let expanded = expand_query("Claude Code hook 机制");
        assert!(expanded.contains(&"Claude".to_string()));
        assert!(expanded.contains(&"Code".to_string()));
        assert!(expanded.contains(&"hook".to_string()));
        assert!(expanded.contains(&"机制".to_string()));
    }

    #[test]
    fn tokenize_mixed_test() {
        let tokens = tokenize_mixed("数据库加密test");
        assert_eq!(tokens, vec!["数据库加密", "test"]);
    }
}
