use super::ParsedMemoryCandidate;

mod external_identity;

#[cfg(test)]
pub(crate) use external_identity::external_candidate_exists;
pub(crate) use external_identity::{
    external_candidate_disposition, insert_external_candidate, ExternalCandidateDisposition,
    ExternalCandidateIdentity, ExternalCandidateInsert, ExternalCandidateOutcome,
};

const ROUTE_CONFIDENCE_HIGH: f64 = 0.95;
const ROUTE_CONFIDENCE_DEFAULT_REPO: f64 = 0.88;
const ROUTE_CONFIDENCE_CONFLICT: f64 = 0.45;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CandidateRoute {
    pub owner_scope: String,
    pub owner_key: String,
    pub target_project: Option<String>,
    pub topic_domain: Option<String>,
    pub routing_confidence: f64,
    pub routing_reason: String,
    pub context_class: String,
}

impl CandidateRoute {
    pub(crate) fn memory_project(&self, source_project: &str) -> String {
        match self.owner_scope.as_str() {
            "repo" => self
                .target_project
                .clone()
                .unwrap_or_else(|| source_project.to_string()),
            "user" => source_project.to_string(),
            "tool" | "domain" | "workstream" | "session" | "workspace" => {
                format!("{}:{}", self.owner_scope, self.owner_key)
            }
            _ => source_project.to_string(),
        }
    }

    pub(crate) fn memory_scope(&self) -> &'static str {
        if self.owner_scope == "user" {
            "global"
        } else {
            "project"
        }
    }

    pub(crate) fn is_repo_owned(&self) -> bool {
        self.owner_scope == "repo"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteKind {
    Repo,
    CodexCli,
    ClaudeCode,
    GhCli,
    GrokApi,
    Macos,
    UserPreference,
}

pub(crate) fn route_candidate<'a>(
    source_project: &str,
    session_id: Option<&str>,
    candidate: &ParsedMemoryCandidate,
    observation_texts: impl IntoIterator<Item = &'a str>,
) -> CandidateRoute {
    let mut haystack = format!(
        "{} {} {} {}",
        candidate.scope, candidate.memory_type, candidate.topic_key, candidate.text
    );
    for text in observation_texts {
        haystack.push(' ');
        haystack.push_str(text);
    }
    let haystack = haystack.to_ascii_lowercase();

    let repo_file_evidence = candidate.scope == "project" && has_repo_file_evidence(&haystack);
    let mut matches = Vec::new();
    if candidate.scope == "global" {
        matches.push(RouteKind::UserPreference);
    }
    if has_any(
        &haystack,
        &[
            "codex",
            "codex-cli",
            "approval policy",
            "sandbox",
            "mcp server",
            "mcp config",
        ],
    ) {
        matches.push(RouteKind::CodexCli);
    }
    if has_any(
        &haystack,
        &[
            "claude code",
            "claude-code",
            "claude hooks",
            "claude desktop",
        ],
    ) {
        matches.push(RouteKind::ClaudeCode);
    }
    if has_any(
        &haystack,
        &[
            "gh cli",
            "github cli",
            "github actions",
            "pull request workflow",
        ],
    ) {
        matches.push(RouteKind::GhCli);
    }
    if has_any(&haystack, &["grok", "xai", "x.ai", "xai api", "grok api"]) {
        matches.push(RouteKind::GrokApi);
    }
    if has_any(
        &haystack,
        &["macos", "tcc", "app bundle", "sparkle", "warp", "launchd"],
    ) {
        matches.push(RouteKind::Macos);
    }

    if matches.is_empty() && candidate.scope == "project" {
        matches.push(RouteKind::Repo);
    }

    if repo_file_evidence
        && matches
            .iter()
            .any(|route| *route != RouteKind::UserPreference)
    {
        matches.clear();
        matches.push(RouteKind::Repo);
    }

    let non_repo_matches = matches
        .iter()
        .copied()
        .filter(|route| *route != RouteKind::Repo)
        .collect::<Vec<_>>();
    if non_repo_matches.len() > 1 {
        return CandidateRoute {
            owner_scope: "session".to_string(),
            owner_key: session_id
                .map(|id| format!("session:{id}"))
                .unwrap_or_else(|| "session:unknown".to_string()),
            target_project: None,
            topic_domain: Some("ambiguous".to_string()),
            routing_confidence: ROUTE_CONFIDENCE_CONFLICT,
            routing_reason: "conflicting deterministic route signals".to_string(),
            context_class: "never_inject".to_string(),
        };
    }

    match matches.first().copied().unwrap_or(RouteKind::Repo) {
        RouteKind::UserPreference => CandidateRoute {
            owner_scope: "user".to_string(),
            owner_key: "user:default".to_string(),
            target_project: None,
            topic_domain: Some("user-preference".to_string()),
            routing_confidence: ROUTE_CONFIDENCE_HIGH,
            routing_reason: "global candidate route".to_string(),
            context_class: "startup_core".to_string(),
        },
        RouteKind::CodexCli => tool_route("codex-cli", "codex-cli", "Codex CLI tool signal"),
        RouteKind::ClaudeCode => tool_route(
            "claude-code",
            "claude-code",
            "Claude Code tool/runtime signal",
        ),
        RouteKind::GhCli => tool_route("gh-cli", "github-workflow", "GitHub CLI/workflow signal"),
        RouteKind::GrokApi => domain_route("grok-api", "grok-api", "Grok/xAI API signal"),
        RouteKind::Macos => domain_route("macos", "macos", "macOS/Warp system signal"),
        RouteKind::Repo => CandidateRoute {
            owner_scope: "repo".to_string(),
            owner_key: source_project.to_string(),
            target_project: Some(source_project.to_string()),
            topic_domain: repo_domain(source_project),
            routing_confidence: ROUTE_CONFIDENCE_DEFAULT_REPO,
            routing_reason: "default project-scoped candidate route".to_string(),
            context_class: "startup_core".to_string(),
        },
    }
}

fn has_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn has_repo_file_evidence(haystack: &str) -> bool {
    has_any(
        haystack,
        &[
            "src/",
            "crates/",
            "tests/",
            "benches/",
            "examples/",
            "cargo.toml",
            "package.json",
            ".rs",
            ".ts",
            ".tsx",
            ".py",
            ".go",
        ],
    )
}

fn tool_route(owner_key: &str, domain: &str, reason: &str) -> CandidateRoute {
    CandidateRoute {
        owner_scope: "tool".to_string(),
        owner_key: owner_key.to_string(),
        target_project: None,
        topic_domain: Some(domain.to_string()),
        routing_confidence: ROUTE_CONFIDENCE_HIGH,
        routing_reason: reason.to_string(),
        context_class: "search_only".to_string(),
    }
}

fn domain_route(owner_key: &str, domain: &str, reason: &str) -> CandidateRoute {
    CandidateRoute {
        owner_scope: "domain".to_string(),
        owner_key: owner_key.to_string(),
        target_project: None,
        topic_domain: Some(domain.to_string()),
        routing_confidence: ROUTE_CONFIDENCE_HIGH,
        routing_reason: reason.to_string(),
        context_class: "search_only".to_string(),
    }
}

fn repo_domain(source_project: &str) -> Option<String> {
    source_project
        .rsplit('/')
        .find(|part| !part.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rusqlite::Connection;

    use super::*;

    fn candidate(text: &str) -> ParsedMemoryCandidate {
        ParsedMemoryCandidate {
            scope: "project".to_string(),
            memory_type: "decision".to_string(),
            topic_key: "topic".to_string(),
            title_override: None,
            text: text.to_string(),
            confidence: 0.9,
            risk_class: "low".to_string(),
            facts: Vec::new(),
        }
    }

    #[test]
    fn routes_codex_sandbox_to_tool_owner() {
        let route = route_candidate(
            "/repo/stash",
            Some("s1"),
            &candidate("Codex CLI sandbox approval mode must stay workspace-write."),
            std::iter::empty(),
        );

        assert_eq!(route.owner_scope, "tool");
        assert_eq!(route.owner_key, "codex-cli");
        assert_eq!(route.memory_project("/repo/stash"), "tool:codex-cli");
        assert_eq!(route.context_class, "search_only");
    }

    #[test]
    fn routes_project_candidate_to_repo_owner() {
        let route = route_candidate(
            "/repo/stash",
            Some("s1"),
            &candidate("Stash drag and drop keeps item ordering in the project UI."),
            std::iter::empty(),
        );

        assert_eq!(route.owner_scope, "repo");
        assert_eq!(route.owner_key, "/repo/stash");
        assert_eq!(route.target_project.as_deref(), Some("/repo/stash"));
    }

    #[test]
    fn repo_file_evidence_overrides_tool_keyword() {
        let route = route_candidate(
            "/repo/stash",
            Some("s1"),
            &candidate("The repo file src/approval_panel.rs renders Codex approval copy."),
            std::iter::empty(),
        );

        assert_eq!(route.owner_scope, "repo");
        assert_eq!(route.owner_key, "/repo/stash");
        assert_eq!(route.target_project.as_deref(), Some("/repo/stash"));
    }

    fn external_insert<'a>(project_id: i64, project: &'a str) -> ExternalCandidateInsert<'a> {
        ExternalCandidateInsert {
            project_id,
            source_project: project,
            scope: "project",
            memory_type: "discovery",
            topic_key: "native-shared-topic",
            text: "The repository uses the shared topic content.",
            confidence: 0.5,
            risk_class: "high",
            source_kind: "claude_native",
            semantic_discriminator_sha256: None,
            owner_scope: "repo",
            owner_key: project,
            target_project: Some(project),
            context_class: "startup_core",
            routing_reason: "external native-memory regression fixture",
            quarantine_match: None,
        }
    }

    #[test]
    fn identical_external_content_in_two_projects_is_not_cross_project_deduped() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let first_project = "/repos/alpha";
        let second_project = "/repos/beta";
        let first_project_id = crate::db::capture::ensure_project_row(&conn, first_project)?;
        let second_project_id = crate::db::capture::ensure_project_row(&conn, second_project)?;

        let first =
            insert_external_candidate(&conn, &external_insert(first_project_id, first_project))?;
        let second =
            insert_external_candidate(&conn, &external_insert(second_project_id, second_project))?;

        assert!(matches!(first, ExternalCandidateOutcome::Inserted { .. }));
        assert!(
            matches!(second, ExternalCandidateOutcome::Inserted { .. }),
            "a different project route must retain its own external candidate"
        );
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_candidates
             WHERE source_kind = 'claude_native'
               AND topic_key = 'native-shared-topic'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 2);
        Ok(())
    }
}
