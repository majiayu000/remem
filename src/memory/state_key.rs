use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeSet;

const MIN_SEMANTIC_SLOT_TERMS: usize = 4;
const MAX_SEMANTIC_SLOT_TERMS: usize = 6;
const CJK_SEMANTIC_SLOT_TERMS: &[(&str, &str)] = &[
    ("三元组", "trigram"),
    ("中文", "cjk"),
    ("全文搜索", "fts5"),
    ("分词器", "tokenizer"),
    ("分词", "tokenizer"),
    ("搜索", "search"),
    ("检索", "retrieval"),
    ("查询", "query"),
    ("数据库", "database"),
    ("加密", "encryption"),
    ("接口", "api"),
    ("钩子", "hook"),
    ("适配器", "adapter"),
    ("评测", "eval"),
    ("基准测试", "benchmark"),
    ("压缩", "compression"),
    ("超时", "timeout"),
    ("工作线程", "worker"),
    ("记忆", "memory"),
    ("捕获", "capture"),
    ("提取", "extraction"),
    ("事实", "fact"),
    ("知识图谱", "knowledge-graph"),
    ("提示词", "prompt"),
    ("发布", "publish"),
    ("部署", "deploy"),
    ("配置", "config"),
    ("端口", "port"),
    ("会话", "session"),
    ("作用域", "scope"),
    ("全局", "global"),
    ("摘要", "summary"),
    ("格式", "format"),
    ("服务器", "server"),
    ("服务", "service"),
    ("性能", "performance"),
    ("上下文", "context"),
    ("竞品", "competitive"),
    ("对比", "comparison"),
    ("偏好", "preference"),
    ("共享", "sharing"),
    ("架构", "architecture"),
    ("设计", "design"),
    ("规则", "rule"),
    ("跨项目", "cross-project"),
    ("候选", "candidate"),
    ("声明", "declaration"),
    ("执行", "execution"),
    ("验证", "verification"),
    ("状态", "status"),
    ("数据", "data"),
    ("代码", "code"),
    ("分离", "separation"),
    ("分开", "separation"),
    ("隔离", "separation"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct StateKeyDecision {
    pub state_key: String,
    pub confidence: f64,
    pub reason: String,
}

impl StateKeyDecision {
    pub fn allows_direct_upsert(&self) -> bool {
        self.reason != "semantic_slot_terms"
    }
}

pub fn derive_state_key(
    memory_type: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
) -> Option<StateKeyDecision> {
    if let Some(topic_key) = stable_state_topic_key(topic_key) {
        return Some(StateKeyDecision {
            state_key: topic_key,
            confidence: 1.0,
            reason: "stable_topic_key".to_string(),
        });
    }

    derive_compat_preference_state_key(memory_type, title, content)
        .or_else(|| derive_semantic_state_key(memory_type, title, content))
}

pub fn current_memory_id(
    conn: &Connection,
    owner_scope: &str,
    owner_key: &str,
    memory_type: &str,
    state_key: &str,
    now_epoch: i64,
) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT m.id
         FROM memory_state_keys sk
         JOIN memories m ON m.id = sk.current_memory_id
         WHERE sk.owner_scope = ?1
           AND sk.owner_key = ?2
           AND sk.memory_type = ?3
           AND sk.state_key = ?4
           AND sk.state_status = 'active'
           AND m.status = 'active'
           AND (m.expires_at_epoch IS NULL OR m.expires_at_epoch > ?5)
         LIMIT 1",
        params![owner_scope, owner_key, memory_type, state_key, now_epoch],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub fn active_memory_ids(
    conn: &Connection,
    owner_scope: &str,
    owner_key: &str,
    memory_type: &str,
    state_key: &str,
    now_epoch: i64,
    require_unexpired: bool,
) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT m.id
         FROM memories m
         JOIN memory_state_keys sk ON sk.id = m.state_key_id
         WHERE sk.owner_scope = ?1
           AND sk.owner_key = ?2
           AND sk.memory_type = ?3
           AND sk.state_key = ?4
           AND sk.state_status = 'active'
           AND m.status = 'active'
           AND (
                ?5 = 0
                OR m.expires_at_epoch IS NULL
                OR m.expires_at_epoch > ?6
           )
         ORDER BY m.updated_at_epoch DESC, m.id DESC",
    )?;
    let rows = stmt.query_map(
        params![
            owner_scope,
            owner_key,
            memory_type,
            state_key,
            if require_unexpired { 1_i64 } else { 0_i64 },
            now_epoch
        ],
        |row| row.get(0),
    )?;
    crate::db::query::collect_rows(rows)
}

pub fn attach_current_memory(
    conn: &Connection,
    memory_id: i64,
    owner_scope: &str,
    owner_key: &str,
    memory_type: &str,
    decision: &StateKeyDecision,
    now_epoch: i64,
) -> Result<i64> {
    let state_key_id = upsert_state_key(
        conn,
        owner_scope,
        owner_key,
        memory_type,
        decision,
        Some(memory_id),
        now_epoch,
    )?;
    conn.execute(
        "UPDATE memories SET state_key_id = ?1 WHERE id = ?2",
        params![state_key_id, memory_id],
    )?;
    Ok(state_key_id)
}

fn upsert_state_key(
    conn: &Connection,
    owner_scope: &str,
    owner_key: &str,
    memory_type: &str,
    decision: &StateKeyDecision,
    current_memory_id: Option<i64>,
    now_epoch: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO memory_state_keys
         (owner_scope, owner_key, memory_type, state_key, state_label, state_status,
          current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?7)
         ON CONFLICT(owner_scope, owner_key, memory_type, state_key)
         DO UPDATE SET
             state_label = COALESCE(excluded.state_label, memory_state_keys.state_label),
             state_status = 'active',
             current_memory_id = COALESCE(excluded.current_memory_id, memory_state_keys.current_memory_id),
             updated_at_epoch = excluded.updated_at_epoch",
        params![
            owner_scope,
            owner_key,
            memory_type,
            decision.state_key,
            decision.state_key.replace('-', " "),
            current_memory_id,
            now_epoch
        ],
    )?;
    conn.query_row(
        "SELECT id FROM memory_state_keys
         WHERE owner_scope = ?1
           AND owner_key = ?2
           AND memory_type = ?3
           AND state_key = ?4",
        params![owner_scope, owner_key, memory_type, decision.state_key],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn stable_state_topic_key(topic_key: Option<&str>) -> Option<String> {
    let topic_key = topic_key?.trim();
    if topic_key.is_empty() || is_hash_like_topic_key(topic_key) {
        return None;
    }
    let slug = crate::memory::promote::slugify_for_topic(topic_key, 120);
    if slug.is_empty() {
        None
    } else {
        Some(slug)
    }
}

fn derive_compat_preference_state_key(
    memory_type: &str,
    title: &str,
    content: &str,
) -> Option<StateKeyDecision> {
    if memory_type != "preference" {
        return None;
    }
    let combined = format!("{title}\n{content}");
    if mentions_verification_status(&combined) && mentions_data_code_separation(&combined) {
        return Some(StateKeyDecision {
            state_key: "verification-status-separation".to_string(),
            confidence: 0.95,
            reason: "preference_domain_verification_status_separation".to_string(),
        });
    }
    if mentions_data_code_separation(&combined) {
        return Some(StateKeyDecision {
            state_key: "data-code-change-separation".to_string(),
            confidence: 0.90,
            reason: "preference_domain_data_code_separation".to_string(),
        });
    }
    if mentions_codesign_binary(&combined) {
        return Some(StateKeyDecision {
            state_key: "local-rust-binary-codesign-after-cp".to_string(),
            confidence: 0.90,
            reason: "preference_domain_codesign_binary".to_string(),
        });
    }

    None
}

fn derive_semantic_state_key(
    memory_type: &str,
    _title: &str,
    content: &str,
) -> Option<StateKeyDecision> {
    let prefix = semantic_slot_prefix(memory_type)?;
    let terms = semantic_slot_terms(content);
    if terms.len() < MIN_SEMANTIC_SLOT_TERMS {
        return None;
    }
    let mut key_terms = terms
        .iter()
        .take(MAX_SEMANTIC_SLOT_TERMS)
        .cloned()
        .collect::<Vec<_>>();
    if terms.len() > MAX_SEMANTIC_SLOT_TERMS {
        key_terms.push(semantic_terms_signature(&terms));
    }
    let raw_key = format!("{prefix}-{}", key_terms.join("-"));
    let state_key = crate::memory::promote::slugify_for_topic(&raw_key, 120);
    if state_key.is_empty() {
        return None;
    }
    Some(StateKeyDecision {
        state_key,
        confidence: 0.82,
        reason: "semantic_slot_terms".to_string(),
    })
}

fn semantic_slot_prefix(memory_type: &str) -> Option<&'static str> {
    match memory_type {
        "architecture" => Some("architecture"),
        "bugfix" => Some("bugfix"),
        "decision" => Some("decision"),
        "discovery" => Some("discovery"),
        "lesson" => Some("lesson"),
        "preference" => Some("preference"),
        "procedure" => Some("procedure"),
        _ => None,
    }
}

fn semantic_slot_terms(text: &str) -> Vec<String> {
    let mut terms = BTreeSet::new();
    for raw in text.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        let Some(term) = normalize_semantic_slot_term(raw) else {
            continue;
        };
        if !is_semantic_slot_stopword(&term) {
            terms.insert(term);
        }
    }
    add_cjk_semantic_slot_terms(text, &mut terms);
    terms.into_iter().collect()
}

fn add_cjk_semantic_slot_terms(text: &str, terms: &mut BTreeSet<String>) {
    if !text.chars().any(is_cjk) {
        return;
    }

    let mut matches = Vec::new();
    for (cjk, canonical) in CJK_SEMANTIC_SLOT_TERMS {
        for (start, _) in text.match_indices(cjk) {
            matches.push((start, start + cjk.len(), cjk.len(), *canonical));
        }
    }
    matches.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));

    let mut claimed = Vec::new();
    for (start, end, _, canonical) in matches {
        if claimed
            .iter()
            .any(|(claimed_start, claimed_end)| start < *claimed_end && end > *claimed_start)
        {
            continue;
        }
        claimed.push((start, end));
        let Some(term) = normalize_semantic_slot_term(canonical) else {
            continue;
        };
        if !is_semantic_slot_stopword(&term) {
            terms.insert(term);
        }
    }
}

fn semantic_terms_signature(terms: &[String]) -> String {
    let joined = terms.join("\0");
    format!(
        "sig{:08x}",
        crate::db::deterministic_hash(joined.as_bytes()) as u32
    )
}

fn normalize_semantic_slot_term(raw: &str) -> Option<String> {
    let mut term = raw.trim().to_ascii_lowercase();
    if term.is_empty() {
        return None;
    }
    term = match term.as_str() {
        "tokenization" | "tokenized" | "tokenize" | "tokenizing" => "tokenizer".to_string(),
        "summaries" => "summary".to_string(),
        "memories" => "memory".to_string(),
        "claims" => "claim".to_string(),
        "candidates" => "candidate".to_string(),
        "decisions" => "decision".to_string(),
        "observations" => "observation".to_string(),
        "indexes" | "indexed" | "indexing" => "index".to_string(),
        "tests" | "tested" | "testing" => "test".to_string(),
        "changes" | "changed" | "changing" => "change".to_string(),
        "updates" | "updated" | "updating" => "update".to_string(),
        "embeddings" => "embedding".to_string(),
        "vectors" => "vector".to_string(),
        "separately" | "separation" | "separate" | "separating" => "separation".to_string(),
        "verification" | "verified" | "verifies" | "verify" => "verification".to_string(),
        "statuses" => "status".to_string(),
        _ => term,
    };
    if term.len() > 4 && term.ends_with('s') && !term.ends_with("ss") {
        term.pop();
    }
    let has_digit = term.chars().any(|ch| ch.is_ascii_digit());
    if term.len() < 3 && !has_digit {
        return None;
    }
    Some(term)
}

fn is_semantic_slot_stopword(term: &str) -> bool {
    matches!(
        term,
        "about"
            | "active"
            | "add"
            | "after"
            | "again"
            | "against"
            | "always"
            | "and"
            | "are"
            | "as"
            | "because"
            | "before"
            | "choose"
            | "current"
            | "default"
            | "disable"
            | "disabled"
            | "does"
            | "enable"
            | "enabled"
            | "for"
            | "from"
            | "has"
            | "have"
            | "into"
            | "keep"
            | "later"
            | "must"
            | "now"
            | "of"
            | "only"
            | "or"
            | "prefer"
            | "record"
            | "remove"
            | "removed"
            | "run"
            | "should"
            | "stop"
            | "support"
            | "supports"
            | "switch"
            | "text"
            | "the"
            | "this"
            | "through"
            | "to"
            | "use"
            | "using"
            | "with"
            | "without"
    )
}

fn is_hash_like_topic_key(topic_key: &str) -> bool {
    let lower = topic_key.to_ascii_lowercase();
    let mut parts = lower.rsplitn(2, ['-', '_']);
    let tail = parts.next().unwrap_or_default();
    let prefix = parts.next().unwrap_or_default();
    tail.len() >= 8
        && tail.chars().all(|ch| ch.is_ascii_hexdigit())
        && matches!(
            prefix,
            "decision"
                | "discovery"
                | "preference"
                | "bugfix"
                | "lesson"
                | "procedure"
                | "architecture"
        )
}

fn mentions_verification_status(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("verification status")
        || lower.contains("verify status")
        || (text.contains("验证") && text.contains("状态"))
}

fn mentions_data_code_separation(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let has_data_code = (lower.contains("data") && lower.contains("code"))
        || (text.contains("数据") && text.contains("代码"));
    let has_separation = lower.contains("separat")
        || lower.contains("distinct")
        || text.contains("分开")
        || text.contains("分离")
        || text.contains("隔离");
    has_data_code && has_separation
}

fn mentions_codesign_binary(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("codesign")
        && (lower.contains("binary")
            || lower.contains("bin/")
            || lower.contains("target/release")
            || lower.contains("cp "))
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_topic_key_is_preserved_as_state_key() {
        let decision = derive_state_key(
            "decision",
            Some("deploy-target"),
            "Deploy target",
            "Deploy to staging.",
        )
        .expect("stable topic key should derive");
        assert_eq!(decision.state_key, "deploy-target");
        assert_eq!(decision.reason, "stable_topic_key");
    }

    #[test]
    fn hash_like_topic_key_uses_ascii_preference_domain() {
        let decision = derive_state_key(
            "preference",
            Some("preference-1234abcd"),
            "Preference",
            "Keep verification status separate from data and code changes.",
        )
        .expect("semantic preference should derive");
        assert_eq!(decision.state_key, "verification-status-separation");
    }

    #[test]
    fn hash_like_decision_uses_semantic_slot_terms() {
        let Some(decision) = derive_state_key(
            "decision",
            Some("decision-deadbeef"),
            "Optimize CJK search",
            "Use FTS5 trigram tokenizer for CJK text search support.",
        ) else {
            panic!("semantic decision should derive");
        };

        assert_eq!(
            decision.state_key,
            "decision-cjk-fts5-search-tokenizer-trigram"
        );
        assert_eq!(decision.reason, "semantic_slot_terms");
    }

    #[test]
    fn hash_like_decision_paraphrase_uses_same_semantic_slot() {
        let Some(first) = derive_state_key(
            "decision",
            Some("decision-11111111"),
            "Optimize CJK search",
            "Use FTS5 trigram tokenizer for CJK text search support.",
        ) else {
            panic!("first decision should derive");
        };
        let Some(second) = derive_state_key(
            "decision",
            Some("decision-22222222"),
            "Refine CJK search",
            "Switch CJK search to FTS5 trigram tokenization.",
        ) else {
            panic!("second decision should derive");
        };

        assert_eq!(first.state_key, second.state_key);
    }

    #[test]
    fn hash_like_cjk_decision_paraphrase_uses_same_semantic_slot() {
        let Some(first) = derive_state_key(
            "decision",
            Some("decision-11111111"),
            "优化中文搜索",
            "使用三元组分词器支持中文搜索。",
        ) else {
            panic!("first CJK decision should derive");
        };
        let Some(second) = derive_state_key(
            "decision",
            Some("decision-22222222"),
            "调整中文搜索",
            "中文搜索改用三元组分词器。",
        ) else {
            panic!("second CJK decision should derive");
        };

        assert_eq!(first.state_key, "decision-cjk-search-tokenizer-trigram");
        assert_eq!(first.state_key, second.state_key);
    }

    #[test]
    fn truncated_semantic_slot_key_includes_full_term_signature() {
        let vector = derive_state_key(
            "decision",
            Some("decision-11111111"),
            "Vector rotation",
            "Use API auth cache migration token rotation vector.",
        )
        .expect("vector decision should derive");
        let workflow = derive_state_key(
            "decision",
            Some("decision-22222222"),
            "Workflow rotation",
            "Use API auth cache migration token rotation workflow.",
        )
        .expect("workflow decision should derive");

        assert!(vector
            .state_key
            .starts_with("decision-api-auth-cache-migration-rotation-token-"));
        assert!(workflow
            .state_key
            .starts_with("decision-api-auth-cache-migration-rotation-token-"));
        assert_ne!(vector.state_key, workflow.state_key);
    }

    #[test]
    fn cjk_semantic_slot_terms_use_longest_non_overlapping_matches() {
        let decision = derive_state_key(
            "decision",
            Some("decision-11111111"),
            "服务器部署",
            "服务器部署配置端口超时性能验证。",
        )
        .expect("CJK decision should derive");

        assert!(decision
            .state_key
            .starts_with("decision-config-deploy-performance-port-server-timeout-"));
        assert!(
            !decision.state_key.contains("-service-"),
            "服务器 should not also contribute the shorter 服务 term: {}",
            decision.state_key
        );
    }

    #[test]
    fn hash_like_topic_key_uses_cjk_preference_domain() {
        let decision = derive_state_key(
            "preference",
            Some("preference-deadbeef"),
            "Preference",
            "验证状态必须和数据、代码变更分开说明。",
        )
        .expect("CJK semantic preference should derive");
        assert_eq!(decision.state_key, "verification-status-separation");
    }

    #[test]
    fn ambiguous_hash_like_non_preference_is_not_invented() {
        assert!(derive_state_key(
            "decision",
            Some("decision-deadbeef"),
            "Decision",
            "A short ambiguous note.",
        )
        .is_none());
    }

    #[test]
    fn current_memory_id_excludes_expired_active_memory() -> Result<()> {
        let conn = rusqlite::Connection::open_in_memory()?;
        crate::memory::tests_helper::setup_memory_schema(&conn);
        let project = "test/proj";
        let state_key = "repo-test-proj-dev-server";
        let memory_id = crate::memory::insert_memory(
            &conn,
            Some("s1"),
            project,
            Some(state_key),
            "Dev server",
            "Local dev server is currently running at localhost:3000.",
            "decision",
            None,
        )?;
        conn.execute(
            "UPDATE memories SET expires_at_epoch = ?1 WHERE id = ?2",
            rusqlite::params![99_i64, memory_id],
        )?;

        let current = current_memory_id(&conn, "repo", project, "decision", state_key, 100)?;

        assert_eq!(current, None);
        Ok(())
    }
}
