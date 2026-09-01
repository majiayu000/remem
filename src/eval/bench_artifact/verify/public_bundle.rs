use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

const MAX_PUBLIC_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

pub(super) fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_PUBLIC_ARTIFACT_BYTES {
        return Err("exceeds the 64 MiB public artifact limit".to_string());
    }
    fs::read(path).map_err(|error| error.to_string())
}

pub(super) fn private_payload_violation(bytes: &[u8]) -> Option<&'static str> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Some("declared text artifact must be UTF-8");
    };
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return private_json_violation(&value);
    }
    private_string_violation(text)
}

fn private_json_violation(value: &Value) -> Option<&'static str> {
    match value {
        Value::String(text) => private_string_violation(text),
        Value::Array(items) => items.iter().find_map(private_json_violation),
        Value::Object(object) => object.values().find_map(private_json_violation),
        Value::Bool(_) | Value::Number(_) | Value::Null => None,
    }
}

pub(super) fn private_string_violation(text: &str) -> Option<&'static str> {
    if text.contains("~/.remem")
        || text.contains("$HOME/.remem")
        || text.contains("${HOME}/.remem")
        || (text.contains("/.remem/")
            && (text.contains("/Users/") || text.contains("/home/") || text.contains("/var/home/")))
    {
        return Some("contains a private remem path");
    }
    let home = dirs::home_dir()?.into_os_string().into_string().ok()?;
    text.starts_with(&home)
        .then_some("contains an absolute path under the current user home")
}

pub(super) fn unreferenced_run_entries(
    root: &Path,
    run_path: &Path,
    artifacts: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let Some(parent) = run_path.parent() else {
        return vec![(
            display(root, run_path),
            "run artifact has no parent directory".to_string(),
        )];
    };
    let mut allowed = artifacts
        .values()
        .map(|path| root.join(path))
        .collect::<BTreeSet<PathBuf>>();
    allowed.insert(run_path.to_path_buf());
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) => {
            return vec![(
                display(root, parent),
                format!("read run artifact directory: {error}"),
            )];
        }
    };
    entries
        .filter_map(|entry| match entry {
            Ok(entry) if allowed.contains(&entry.path()) => None,
            Ok(entry) => Some((
                display(root, &entry.path()),
                "unreferenced file in v2 run artifact directory".to_string(),
            )),
            Err(error) => Some((
                display(root, parent),
                format!("read run artifact directory entry: {error}"),
            )),
        })
        .collect()
}

fn display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
