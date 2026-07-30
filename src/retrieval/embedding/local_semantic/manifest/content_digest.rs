use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use super::super::{
    is_sha256_hex, LocalEmbeddingPreset, LocalModelFile, LocalModelManifest, LocalModelSymlink,
};

const CONTENT_DIGEST_DOMAIN: &[u8] = b"remem-local-embedding-content-digest-v1";

pub(in crate::retrieval::embedding::local_semantic) fn model_content_sha256(
    manifest: &LocalModelManifest,
) -> Result<String> {
    let preset = LocalEmbeddingPreset::parse(&manifest.preset)?;
    let logical_files = logical_runtime_files(manifest, preset)?;
    let mut hasher = Sha256::new();
    update_field(&mut hasher, CONTENT_DIGEST_DOMAIN)?;
    update_field(&mut hasher, manifest.preset.as_bytes())?;
    update_field(&mut hasher, manifest.model_id.as_bytes())?;
    update_field(&mut hasher, manifest.upstream_model.as_bytes())?;
    update_u64(&mut hasher, manifest.dimensions as u64);
    update_field(&mut hasher, manifest.runtime.as_bytes())?;
    update_u64(
        &mut hasher,
        u64::try_from(logical_files.len()).context("count logical local model runtime files")?,
    );
    for logical in logical_files {
        update_field(&mut hasher, logical.name.as_bytes())?;
        update_u64(&mut hasher, logical.file.bytes);
        update_field(&mut hasher, logical.file.sha256.as_bytes())?;
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

struct LogicalRuntimeFile<'a> {
    name: &'static str,
    file: &'a LocalModelFile,
}

fn logical_runtime_files(
    manifest: &LocalModelManifest,
    preset: LocalEmbeddingPreset,
) -> Result<Vec<LogicalRuntimeFile<'_>>> {
    let repo_prefix = format!("{}/snapshots/", preset.cache_repo_dir());
    let file_by_path = manifest
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect::<HashMap<_, _>>();
    let symlink_by_path = manifest
        .symlinks
        .iter()
        .map(|symlink| (symlink.path.as_str(), symlink))
        .collect::<HashMap<_, _>>();
    let mut revision = None::<&str>;
    let mut logical_files = Vec::new();

    for logical_name in preset.required_runtime_files() {
        let matches = matching_snapshot_paths(
            &repo_prefix,
            logical_name,
            manifest.files.iter().map(|file| file.path.as_str()),
            manifest
                .symlinks
                .iter()
                .map(|symlink| symlink.path.as_str()),
        );
        if matches.len() != 1 {
            bail!(
                "local embedding manifest must bind logical runtime file {logical_name} exactly once; found {} entries",
                matches.len()
            );
        }
        let (path, current_revision) = matches[0];
        if let Some(expected_revision) = revision {
            if current_revision != expected_revision {
                bail!(
                    "local embedding manifest mixes runtime revisions {expected_revision} and {current_revision}"
                );
            }
        } else {
            revision = Some(current_revision);
        }
        let file = match (file_by_path.get(path), symlink_by_path.get(path)) {
            (Some(file), None) => *file,
            (None, Some(symlink)) => resolve_manifest_symlink_file(&file_by_path, symlink)?,
            _ => bail!("ambiguous logical runtime manifest path {path}"),
        };
        if !is_sha256_hex(&file.sha256) {
            bail!(
                "logical runtime file {logical_name} has invalid SHA-256 {}",
                file.sha256
            );
        }
        logical_files.push(LogicalRuntimeFile {
            name: logical_name,
            file,
        });
    }
    logical_files.sort_by(|left, right| left.name.cmp(right.name));
    Ok(logical_files)
}

fn matching_snapshot_paths<'a>(
    repo_prefix: &str,
    logical_name: &str,
    file_paths: impl Iterator<Item = &'a str>,
    symlink_paths: impl Iterator<Item = &'a str>,
) -> Vec<(&'a str, &'a str)> {
    file_paths
        .chain(symlink_paths)
        .filter_map(|path| {
            let suffix = path.strip_prefix(repo_prefix)?;
            let (revision, candidate_name) = suffix.split_once('/')?;
            (candidate_name == logical_name).then_some((path, revision))
        })
        .collect()
}

fn resolve_manifest_symlink_file<'a>(
    file_by_path: &HashMap<&str, &'a LocalModelFile>,
    symlink: &LocalModelSymlink,
) -> Result<&'a LocalModelFile> {
    file_by_path
        .get(symlink.resolved_path.as_str())
        .copied()
        .with_context(|| {
            format!(
                "logical runtime symlink {} resolves to unlisted file {}",
                symlink.path, symlink.resolved_path
            )
        })
}

fn update_field(hasher: &mut Sha256, value: &[u8]) -> Result<()> {
    let len =
        u64::try_from(value.len()).context("encode local model content digest field length")?;
    update_u64(hasher, len);
    hasher.update(value);
    Ok(())
}

fn update_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(value.to_be_bytes());
}
