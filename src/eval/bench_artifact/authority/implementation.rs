use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::super::types::ImplementationAuthorityBinding;

const PRODUCTION_PATHSPEC_CONTRACT: &str = "eval/production-input-pathspec-v1.json";

pub(super) fn runtime_binding() -> ImplementationAuthorityBinding {
    let build_git_sha = option_env!("REMEM_BUILD_GIT_SHA");
    let build_source_dirty = option_env!("REMEM_BUILD_SOURCE_DIRTY").and_then(parse_bool);
    let build_tree = option_env!("REMEM_BUILD_PRODUCTION_INPUT_TREE_SHA256");
    let embedded_pathspec = option_env!("REMEM_BUILD_PRODUCTION_PATHSPEC_JSON");
    let declared_pathspec_hash = option_env!("REMEM_BUILD_PRODUCTION_PATHSPEC_SHA256");
    let pathspec = embedded_pathspec.and_then(parse_production_pathspec);
    let pathspec_hash =
        embedded_pathspec
            .zip(declared_pathspec_hash)
            .and_then(|(contract, declared)| {
                let computed = format!("{:x}", Sha256::digest(contract.as_bytes()));
                (normalize_lower_hex(declared, 64).as_deref() == Some(computed.as_str()))
                    .then_some(computed)
            });
    let repository_root = repository_root();
    let checkout_git_sha = repository_root
        .as_deref()
        .and_then(|root| git_stdout(root, &["rev-parse", "--verify", "HEAD^{commit}"]));
    let checkout_source_dirty = repository_root.as_deref().and_then(checkout_dirty);
    let checkout_tree = repository_root
        .as_deref()
        .zip(pathspec.as_deref())
        .and_then(|(root, paths)| production_input_tree_sha256(root, paths));

    let mut binding = binding_from_parts(
        build_git_sha,
        checkout_git_sha.as_deref(),
        build_source_dirty,
        checkout_source_dirty,
        build_tree,
        checkout_tree.as_deref(),
        pathspec_hash.as_deref(),
    );
    if repository_root.is_none() {
        binding
            .diagnostics
            .push("checkout repository root is unavailable".to_string());
    }
    if pathspec.is_none() || pathspec_hash.is_none() {
        binding
            .diagnostics
            .push("embedded production pathspec identity is unavailable or malformed".to_string());
    }
    binding
}

pub(in crate::eval::bench_artifact) fn binding_from_parts(
    build_git_sha: Option<&str>,
    checkout_git_sha: Option<&str>,
    build_source_dirty: Option<bool>,
    checkout_source_dirty: Option<bool>,
    build_tree: Option<&str>,
    checkout_tree: Option<&str>,
    production_pathspec_sha256: Option<&str>,
) -> ImplementationAuthorityBinding {
    let build_git_sha = build_git_sha.and_then(|value| normalize_lower_hex(value, 40));
    let checkout_git_sha = checkout_git_sha.and_then(|value| normalize_lower_hex(value, 40));
    let build_tree = build_tree.and_then(|value| normalize_lower_hex(value, 64));
    let checkout_tree = checkout_tree.and_then(|value| normalize_lower_hex(value, 64));
    let production_pathspec_sha256 =
        production_pathspec_sha256.and_then(|value| normalize_lower_hex(value, 64));
    let executable_source_equivalent = build_git_sha.is_some()
        && checkout_git_sha.is_some()
        && build_tree.is_some()
        && build_tree == checkout_tree
        && production_pathspec_sha256.is_some()
        && build_source_dirty == Some(false)
        && checkout_source_dirty == Some(false);
    let mut diagnostics = Vec::new();
    if build_git_sha.is_none() || checkout_git_sha.is_none() {
        diagnostics.push("build or checkout Git SHA is unavailable or malformed".to_string());
    }
    if build_tree.is_none() || checkout_tree.is_none() {
        diagnostics.push(
            "build or checkout production-input tree is unavailable or malformed".to_string(),
        );
    }
    if production_pathspec_sha256.is_none() {
        diagnostics.push("production pathspec hash is unavailable or malformed".to_string());
    }
    if build_source_dirty != Some(false) || checkout_source_dirty != Some(false) {
        diagnostics.push("build or checkout source state is not clean".to_string());
    }
    if !executable_source_equivalent {
        diagnostics.push("build and checkout executable sources are not equivalent".to_string());
    }
    ImplementationAuthorityBinding {
        build_git_sha,
        checkout_git_sha,
        build_source_dirty,
        checkout_source_dirty,
        build_production_input_tree_sha256: build_tree,
        checkout_production_input_tree_sha256: checkout_tree,
        production_pathspec_sha256,
        executable_source_equivalent,
        diagnostics,
    }
}

pub(crate) fn production_input_tree_sha256(root: &Path, paths: &[String]) -> Option<String> {
    let mut args = vec!["ls-files", "-s", "--"];
    args.extend(paths.iter().map(String::as_str));
    let output = crate::git_util::git_output_soft(root, &args)?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    Some(format!("{:x}", Sha256::digest(&output.stdout)))
}

pub(super) fn production_input_tree_sha256_for_commit(commit: &str) -> Option<String> {
    let root = repository_root()?;
    let object = format!("{commit}^{{commit}}");
    if git_stdout(&root, &["rev-parse", "--verify", &object])? != commit {
        return None;
    }
    let paths = parse_production_pathspec(include_str!(
        "../../../../eval/production-input-pathspec-v1.json"
    ))?;
    let mut args = vec!["ls-tree", "-r", commit, "--"];
    args.extend(paths.iter().map(String::as_str));
    let output = crate::git_util::git_output_soft(&root, &args)?;
    let text = output
        .status
        .success()
        .then(|| std::str::from_utf8(&output.stdout).ok())??;
    let mut staged = Vec::new();
    for line in text.lines() {
        let (metadata, path) = line.split_once('\t')?;
        let mut fields = metadata.split_whitespace();
        let mode = fields.next()?;
        fields.next()?;
        let object = fields.next()?;
        staged.extend_from_slice(format!("{mode} {object} 0\t{path}\n").as_bytes());
    }
    (!staged.is_empty()).then(|| format!("{:x}", Sha256::digest(staged)))
}

fn repository_root() -> Option<PathBuf> {
    git_stdout(Path::new("."), &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}

fn checkout_dirty(root: &Path) -> Option<bool> {
    let output = crate::git_util::git_output_soft(
        root,
        &["status", "--porcelain", "--untracked-files=normal"],
    )?;
    output.status.success().then_some(!output.stdout.is_empty())
}

fn git_stdout(root: &Path, args: &[&str]) -> Option<String> {
    let output = crate::git_util::git_output_soft(root, args)?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn parse_production_pathspec(embedded: &str) -> Option<Vec<String>> {
    let value: Value = serde_json::from_str(embedded).ok()?;
    if value.get("schema_version").and_then(Value::as_u64) != Some(1)
        || value.get("contract_id").and_then(Value::as_str)
            != Some("remem-production-input-pathspec-v1")
    {
        return None;
    }
    let paths = value
        .get("paths")?
        .as_array()?
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()?;
    if paths.is_empty()
        || paths.iter().any(|path| path.is_empty())
        || paths.iter().any(|path| {
            let path = Path::new(path);
            path.is_absolute()
                || path
                    .components()
                    .any(|component| matches!(component, Component::ParentDir))
        })
        || paths.iter().any(|path| {
            path.starts_with(':')
                || path
                    .chars()
                    .any(|character| matches!(character, '*' | '?' | '['))
        })
        || paths.iter().collect::<BTreeSet<_>>().len() != paths.len()
        || !paths.contains(&PRODUCTION_PATHSPEC_CONTRACT)
    {
        return None;
    }
    Some(paths.into_iter().map(str::to_string).collect())
}

fn normalize_lower_hex(value: &str, len: usize) -> Option<String> {
    (value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then(|| value.to_string())
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_production_pathspec;

    #[test]
    fn commit_tree_matches_the_same_clean_checkout_tree() {
        let root = super::repository_root().expect("repository root");
        let head = super::git_stdout(&root, &["rev-parse", "--verify", "HEAD^{commit}"])
            .expect("HEAD commit");
        let paths = super::parse_production_pathspec(include_str!(
            "../../../../eval/production-input-pathspec-v1.json"
        ))
        .expect("production pathspec");
        let checkout = super::production_input_tree_sha256(&root, &paths).expect("checkout tree");

        assert_eq!(
            super::production_input_tree_sha256_for_commit(&head),
            Some(checkout)
        );
        assert_eq!(
            super::production_input_tree_sha256_for_commit(&"f".repeat(40)),
            None
        );
    }

    #[test]
    fn production_pathspec_rejects_git_magic_and_wildcard_paths() {
        assert!(parse_production_pathspec(include_str!(
            "../../../../eval/production-input-pathspec-v1.json"
        ))
        .is_some());

        for dangerous_path in [":(exclude)src", "src/**"] {
            let contract = serde_json::json!({
                "schema_version": 1,
                "contract_id": "remem-production-input-pathspec-v1",
                "paths": [dangerous_path, "eval/production-input-pathspec-v1.json"]
            });

            assert!(
                parse_production_pathspec(&contract.to_string()).is_none(),
                "dangerous pathspec must be rejected: {dangerous_path}"
            );
        }
    }
}
