use anyhow::Result;

use super::candidates::{Cluster, MemoryCandidate};
use super::constants::DREAM_PROMPT;

#[derive(Debug)]
pub(super) enum MergeDecision {
    Merge(MergeResult),
    NoMerge,
}

#[derive(Debug)]
pub(super) struct MergeResult {
    pub topic_key: String,
    pub memory_type: String,
    pub title: String,
    pub content: String,
    pub superseded_ids: Vec<i64>,
}

pub(super) async fn merge_cluster(cluster: &Cluster, project: &str) -> Result<MergeDecision> {
    let user_message = build_user_message(&cluster.members);

    let response = crate::ai::call_ai(
        DREAM_PROMPT,
        &user_message,
        crate::ai::UsageContext {
            project: Some(project),
            operation: "dream",
        },
    )
    .await?;

    Ok(filter_superseded_ids(parse_response(&response), cluster))
}

fn filter_superseded_ids(decision: MergeDecision, cluster: &Cluster) -> MergeDecision {
    let MergeDecision::Merge(mut result) = decision else {
        return decision;
    };
    let member_ids: std::collections::HashSet<i64> = cluster.members.iter().map(|m| m.id).collect();
    let before = result.superseded_ids.len();
    result.superseded_ids.retain(|id| member_ids.contains(id));
    let dropped = before - result.superseded_ids.len();
    if dropped > 0 {
        crate::log::warn(
            "dream",
            &format!(
                "dropped {} hallucinated superseded_id(s) not in cluster",
                dropped
            ),
        );
    }
    if result.superseded_ids.is_empty() {
        crate::log::warn(
            "dream",
            "rejecting merge with no valid superseded_id(s) after filtering",
        );
        return MergeDecision::NoMerge;
    }
    MergeDecision::Merge(result)
}

fn build_user_message(members: &[MemoryCandidate]) -> String {
    let mut msg = String::from("Merge these memory entries:\n\n");
    for m in members {
        msg.push_str(&format!(
            "<entry id=\"{}\" type=\"{}\" topic_key=\"{}\">\n<title>{}</title>\n<content>{}</content>\n</entry>\n\n",
            m.id,
            xml_escape(&m.memory_type),
            xml_escape(m.topic_key.as_deref().unwrap_or("")),
            xml_escape(&m.title),
            xml_escape(&m.content),
        ));
    }
    msg
}

fn parse_response(response: &str) -> MergeDecision {
    if response.contains("<no_merge") {
        return MergeDecision::NoMerge;
    }

    let topic_key = extract_tag(response, "topic_key").unwrap_or_default();
    let memory_type = extract_tag(response, "type").unwrap_or_default();
    let title = extract_tag(response, "title").unwrap_or_default();
    let content = extract_tag(response, "content").unwrap_or_default();
    let supersedes_raw = extract_tag(response, "supersedes").unwrap_or_default();

    if topic_key.is_empty() || title.is_empty() || content.is_empty() {
        return MergeDecision::NoMerge;
    }

    let superseded_ids: Vec<i64> = supersedes_raw
        .split_whitespace()
        .filter_map(|s| s.parse::<i64>().ok())
        .collect();
    if superseded_ids.is_empty() {
        return MergeDecision::NoMerge;
    }

    MergeDecision::Merge(MergeResult {
        topic_key,
        memory_type: if memory_type.is_empty() {
            "discovery".to_owned()
        } else {
            memory_type
        },
        title,
        content,
        superseded_ids,
    })
}

fn extract_tag(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(text[start..end].trim().to_owned())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_merge() {
        let response = r#"<memory>
<topic_key>auth-design</topic_key>
<type>decision</type>
<title>Auth middleware uses JWT</title>
<content>Use JWT for stateless auth. Previously: session cookies.</content>
<supersedes>42 17</supersedes>
</memory>"#;
        match parse_response(response) {
            MergeDecision::Merge(r) => {
                assert_eq!(r.topic_key, "auth-design");
                assert_eq!(r.memory_type, "decision");
                assert_eq!(r.superseded_ids, vec![42, 17]);
            }
            MergeDecision::NoMerge => panic!("expected Merge"),
        }
    }

    #[test]
    fn test_parse_no_merge() {
        let response = r#"<no_merge reason="entries cover different topics"/>"#;
        assert!(matches!(parse_response(response), MergeDecision::NoMerge));
    }

    #[test]
    fn test_parse_missing_fields_becomes_no_merge() {
        let response = "<memory><topic_key>k</topic_key></memory>";
        assert!(matches!(parse_response(response), MergeDecision::NoMerge));
    }

    #[test]
    fn test_filter_superseded_ids_drops_hallucinated() {
        let cluster = Cluster {
            members: vec![
                MemoryCandidate {
                    id: 10,
                    topic_key: Some("k".into()),
                    title: "t".into(),
                    content: "c".into(),
                    memory_type: "decision".into(),
                    updated_at_epoch: 0,
                },
                MemoryCandidate {
                    id: 20,
                    topic_key: Some("k".into()),
                    title: "t".into(),
                    content: "c".into(),
                    memory_type: "decision".into(),
                    updated_at_epoch: 0,
                },
            ],
        };
        let decision = MergeDecision::Merge(MergeResult {
            topic_key: "k".into(),
            memory_type: "decision".into(),
            title: "T".into(),
            content: "C".into(),
            // 99999 is hallucinated; 10 and 20 are valid cluster members
            superseded_ids: vec![10, 99999, 20],
        });
        match filter_superseded_ids(decision, &cluster) {
            MergeDecision::Merge(r) => assert_eq!(r.superseded_ids, vec![10, 20]),
            MergeDecision::NoMerge => panic!("expected Merge"),
        }
    }

    #[test]
    fn test_filter_superseded_ids_rejects_empty_after_filter() {
        let cluster = Cluster {
            members: vec![MemoryCandidate {
                id: 10,
                topic_key: Some("k".into()),
                title: "t".into(),
                content: "c".into(),
                memory_type: "decision".into(),
                updated_at_epoch: 0,
            }],
        };
        let decision = MergeDecision::Merge(MergeResult {
            topic_key: "k".into(),
            memory_type: "decision".into(),
            title: "T".into(),
            content: "C".into(),
            superseded_ids: vec![99999],
        });
        assert!(matches!(
            filter_superseded_ids(decision, &cluster),
            MergeDecision::NoMerge
        ));
    }

    #[test]
    fn test_filter_superseded_ids_no_merge_passthrough() {
        let cluster = Cluster { members: vec![] };
        assert!(matches!(
            filter_superseded_ids(MergeDecision::NoMerge, &cluster),
            MergeDecision::NoMerge
        ));
    }

    #[test]
    fn test_parse_empty_supersedes_becomes_no_merge() {
        let response = r#"<memory>
<topic_key>k</topic_key>
<type>decision</type>
<title>T</title>
<content>C</content>
<supersedes></supersedes>
</memory>"#;
        assert!(matches!(parse_response(response), MergeDecision::NoMerge));
    }
}
