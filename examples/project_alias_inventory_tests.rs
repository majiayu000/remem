use super::*;

#[test]
fn remote_normalization_matches_https_and_ssh() {
    assert_eq!(
        normalize_remote("https://github.com/Org/Repo.git"),
        Some("github.com/Org/Repo".to_string())
    );
    assert_eq!(
        normalize_remote("git@github.com:Org/Repo.git"),
        Some("github.com/Org/Repo".to_string())
    );
    assert_eq!(normalize_remote(""), None);
}

#[test]
fn inventory_reads_only_allowlisted_path_columns() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(
        "CREATE TABLE workspaces(root_path TEXT, git_remote TEXT);
         CREATE TABLE memories(
             project TEXT,
             source_project TEXT,
             owner_scope TEXT,
             owner_key TEXT,
             content TEXT
         );
         INSERT INTO workspaces VALUES('/old/repo', 'https://github.com/o/r.git');
         INSERT INTO memories VALUES('/old/repo', '/old/repo', 'repo', '/old/repo', 'secret');
         INSERT INTO memories VALUES('/old/repo', '/old/repo', 'user', 'not-a-path', 'secret');",
    )?;
    let observed = load_observed_paths(&conn)?;
    let old = observed.get("/old/repo").expect("old path");
    assert_eq!(old.stored_remotes.len(), 1);
    assert!(old.surfaces.iter().any(|s| s.column == "project"));
    assert!(old.surfaces.iter().any(|s| s.column == "owner_key"));
    assert!(!observed.contains_key("not-a-path"));
    assert!(!observed.contains_key("secret"));
    Ok(())
}

#[test]
fn missing_path_with_one_remote_match_is_moved() {
    let mut observed = BTreeMap::new();
    observed.insert(
        "/old/repo".to_string(),
        ObservedPath {
            surfaces: Vec::new(),
            stored_remotes: BTreeSet::from(["github.com/o/r".to_string()]),
            commit_shas: BTreeSet::new(),
        },
    );
    observed.insert("/new/repo".to_string(), ObservedPath::default());
    let rows = classify_paths(
        observed,
        |path| match path {
            "/new/repo" => LiveEvidence {
                exists: true,
                canonical_root: Some("/new/repo".to_string()),
                remote: Some("github.com/o/r".to_string()),
            },
            _ => LiveEvidence::default(),
        },
        |_, _| false,
    );
    let old = rows.iter().find(|row| row.path == "/old/repo").unwrap();
    assert_eq!(old.classification, Classification::Moved);
    assert_eq!(old.canonical_target.as_deref(), Some("/new/repo"));
}

#[test]
fn multiple_live_roots_for_remote_abstain_as_ambiguous() {
    let mut observed = BTreeMap::new();
    observed.insert(
        "/old/repo".to_string(),
        ObservedPath {
            surfaces: Vec::new(),
            stored_remotes: BTreeSet::from(["github.com/o/r".to_string()]),
            commit_shas: BTreeSet::new(),
        },
    );
    observed.insert("/worktree/a".to_string(), ObservedPath::default());
    observed.insert("/worktree/b".to_string(), ObservedPath::default());
    let rows = classify_paths(
        observed,
        |path| {
            if path.starts_with("/worktree/") {
                LiveEvidence {
                    exists: true,
                    canonical_root: Some(path.to_string()),
                    remote: Some("github.com/o/r".to_string()),
                }
            } else {
                LiveEvidence::default()
            }
        },
        |_, _| false,
    );
    let old = rows.iter().find(|row| row.path == "/old/repo").unwrap();
    assert_eq!(old.classification, Classification::Ambiguous);
    assert!(old.canonical_target.is_none());
}

#[test]
fn shared_commit_evidence_links_missing_path_without_stored_remote() {
    let mut observed = BTreeMap::new();
    observed.insert(
        "/old/repo".to_string(),
        ObservedPath {
            surfaces: Vec::new(),
            stored_remotes: BTreeSet::new(),
            commit_shas: BTreeSet::from(["abc123".to_string()]),
        },
    );
    observed.insert(
        "/new/repo".to_string(),
        ObservedPath {
            surfaces: Vec::new(),
            stored_remotes: BTreeSet::new(),
            commit_shas: BTreeSet::from(["abc123".to_string(), "def456".to_string()]),
        },
    );
    let rows = classify_paths(
        observed,
        |path| match path {
            "/new/repo" => LiveEvidence {
                exists: true,
                canonical_root: Some("/new/repo".to_string()),
                remote: Some("github.com/o/r".to_string()),
            },
            _ => LiveEvidence::default(),
        },
        |_, _| false,
    );
    let old = rows.iter().find(|row| row.path == "/old/repo").unwrap();
    assert_eq!(old.classification, Classification::Moved);
    assert_eq!(old.canonical_target.as_deref(), Some("/new/repo"));
    assert_eq!(old.shared_commit_count, 1);
}

#[test]
fn live_repository_commit_proof_links_same_name_missing_path() {
    let mut observed = BTreeMap::new();
    observed.insert(
        "/old/parent/repo".to_string(),
        ObservedPath {
            surfaces: Vec::new(),
            stored_remotes: BTreeSet::new(),
            commit_shas: BTreeSet::from(["abc123".to_string()]),
        },
    );
    observed.insert("/new/parent/repo".to_string(), ObservedPath::default());
    let rows = classify_paths(
        observed,
        |path| match path {
            "/new/parent/repo" => LiveEvidence {
                exists: true,
                canonical_root: Some("/new/parent/repo".to_string()),
                remote: Some("github.com/o/r".to_string()),
            },
            _ => LiveEvidence::default(),
        },
        |root, sha| root == "/new/parent/repo" && sha == "abc123",
    );
    let old = rows
        .iter()
        .find(|row| row.path == "/old/parent/repo")
        .unwrap();
    assert_eq!(old.classification, Classification::Moved);
    assert_eq!(old.canonical_target.as_deref(), Some("/new/parent/repo"));
    assert_eq!(old.shared_commit_count, 1);
}

#[test]
fn report_digest_is_deterministic() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(
        "PRAGMA user_version = 80;
         CREATE TABLE workspaces(root_path TEXT, git_remote TEXT);
         INSERT INTO workspaces VALUES('/repo', 'https://github.com/o/r.git');",
    )?;
    let resolver = |path: &str| LiveEvidence {
        exists: true,
        canonical_root: Some(path.to_string()),
        remote: Some("github.com/o/r".to_string()),
    };
    let first = build_report(&conn, resolver, |_, _| false)?;
    let second = build_report(&conn, resolver, |_, _| false)?;
    assert_eq!(first.inventory_sha256, second.inventory_sha256);
    assert_eq!(first.paths, second.paths);
    Ok(())
}
