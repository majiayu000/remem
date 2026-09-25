use super::*;

// Catches treating an explicit archive root's name as a descendant exclusion.
#[test]
fn explicit_subagents_named_root_is_scanned_but_nested_subagents_are_skipped() {
    let root =
        TempRoot::new_with_components("explicit-root", &["subagents"], InstallHost::CodexCli);
    let expected = root.write("main.jsonl", "{}\n");
    root.write("subagents/skipped.jsonl", "{}\n");
    let (files, errors) = discover_transcript_files(&root.scan_root("archive"));
    assert_eq!(files, vec![expected]);
    assert!(errors.is_empty());
}

#[cfg(unix)]
#[test]
fn explicit_symlink_root_preserves_lexical_identity_and_skips_child_links() {
    let root = TempRoot::new_unclassified("symlink-root");
    root.write("actual/main.jsonl", "{}\n");
    std::os::unix::fs::symlink(root.path.join("actual"), root.path.join("alias")).unwrap();
    std::os::unix::fs::symlink(root.path.join("actual"), root.path.join("actual/loop")).unwrap();
    let mut scan_root = root.scan_root("archive");
    scan_root.path = root.path.join("alias");
    let (files, errors) = discover_transcript_files(&scan_root);
    assert_eq!(files, vec![root.path.join("alias/main.jsonl")]);
    assert!(errors.is_empty());
}
