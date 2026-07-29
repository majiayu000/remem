use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::super::{
    checked_relative_path, sha256_file, source_sha256_from_hf_blob_path, LocalEmbeddingPreset,
    LocalModelFile, LocalModelManifest, LocalModelSymlink,
};

pub(super) fn collect_model_artifacts(
    install_dir: &Path,
    preset: LocalEmbeddingPreset,
) -> Result<(Vec<LocalModelFile>, Vec<LocalModelSymlink>)> {
    let canonical_install_dir = std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize {}", install_dir.display()))?;
    let repo_dir = preset.cache_repo_dir();
    let ref_relative = format!("{repo_dir}/refs/main");
    let ref_path = canonical_install_dir.join(checked_relative_path(&ref_relative)?);
    let revision = read_active_revision(&canonical_install_dir, &ref_relative)?;

    let mut files = HashMap::<String, LocalModelFile>::new();
    let mut symlinks = Vec::new();
    insert_regular_file(&canonical_install_dir, &ref_path, &mut files)?;

    for runtime_file in preset.required_runtime_files() {
        let relative = format!("{repo_dir}/snapshots/{revision}/{runtime_file}");
        let path = canonical_install_dir.join(checked_relative_path(&relative)?);
        let metadata =
            std::fs::symlink_metadata(&path).with_context(|| format!("stat {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            let symlink =
                collect_snapshot_symlink(&canonical_install_dir, &path, preset, &revision)?;
            let resolved_path =
                canonical_install_dir.join(checked_relative_path(&symlink.resolved_path)?);
            insert_regular_file(&canonical_install_dir, &resolved_path, &mut files)?;
            symlinks.push(symlink);
        } else if metadata.file_type().is_file() {
            insert_regular_file(&canonical_install_dir, &path, &mut files)?;
        } else {
            bail!("required model path is not a regular file or symlink: {relative}");
        }
    }

    let mut files = files.into_values().collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    symlinks.sort_by(|left, right| left.path.cmp(&right.path));
    ensure_unique_paths(files.iter().map(|file| file.path.as_str()), "file")?;
    ensure_unique_paths(
        symlinks.iter().map(|symlink| symlink.path.as_str()),
        "symlink",
    )?;
    Ok((files, symlinks))
}

#[cfg(feature = "local-onnx")]
pub(super) fn verified_runtime_file<'a>(
    install_dir: &Path,
    manifest: &'a LocalModelManifest,
    preset: LocalEmbeddingPreset,
    runtime_file: &str,
) -> Result<(&'a LocalModelFile, PathBuf)> {
    if !preset
        .required_runtime_files()
        .any(|required| required == runtime_file)
    {
        bail!(
            "runtime file {runtime_file} is not declared for local embedding preset {}",
            preset.label()
        );
    }
    let repo_dir = preset.cache_repo_dir();
    let revision = active_revision_for_manifest(install_dir, manifest, preset)?;
    let snapshot_relative = format!("{repo_dir}/snapshots/{revision}/{runtime_file}");
    let (file, relative_path) = if let Some(file) = manifest
        .files
        .iter()
        .find(|file| file.path == snapshot_relative)
    {
        (file, file.path.as_str())
    } else {
        let symlink = manifest
            .symlinks
            .iter()
            .find(|symlink| symlink.path == snapshot_relative)
            .with_context(|| {
                format!(
                    "verified manifest is missing runtime file {runtime_file} for {}",
                    preset.label()
                )
            })?;
        let file = manifest
            .files
            .iter()
            .find(|file| file.path == symlink.resolved_path)
            .with_context(|| {
                format!(
                    "verified manifest symlink {} has unlisted target {}",
                    symlink.path, symlink.resolved_path
                )
            })?;
        (file, file.path.as_str())
    };
    let (path, _) = canonical_regular_path(install_dir, relative_path)?;
    Ok((file, path))
}

pub(super) fn verify_runtime_layout(
    install_dir: &Path,
    manifest: &LocalModelManifest,
    preset: LocalEmbeddingPreset,
) -> Result<()> {
    let revision = active_revision_for_manifest(install_dir, manifest, preset)?;
    verify_runtime_layout_at_revision(manifest, preset, &revision)
}

pub(super) fn verify_runtime_layout_at_revision(
    manifest: &LocalModelManifest,
    preset: LocalEmbeddingPreset,
    revision: &str,
) -> Result<()> {
    let repo_dir = preset.cache_repo_dir();
    let file_paths = manifest
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<HashSet<_>>();
    let symlink_paths = manifest
        .symlinks
        .iter()
        .map(|symlink| symlink.path.as_str())
        .collect::<HashSet<_>>();
    for runtime_file in preset.required_runtime_files() {
        let path = format!("{repo_dir}/snapshots/{revision}/{runtime_file}");
        if !file_paths.contains(path.as_str()) && !symlink_paths.contains(path.as_str()) {
            bail!(
                "local embedding manifest is missing required runtime file {runtime_file} for {}",
                preset.label()
            );
        }
    }
    for symlink in &manifest.symlinks {
        let expected_prefix = format!("{repo_dir}/snapshots/{revision}/");
        if !symlink.path.starts_with(&expected_prefix) {
            bail!(
                "manifest symlink {} is outside active snapshot {}",
                symlink.path,
                revision
            );
        }
        let expected_blob_prefix = format!("{repo_dir}/blobs/");
        if !symlink.resolved_path.starts_with(&expected_blob_prefix)
            || symlink.resolved_path[expected_blob_prefix.len()..].contains('/')
        {
            bail!(
                "manifest symlink {} resolves outside repository blobs: {}",
                symlink.path,
                symlink.resolved_path
            );
        }
    }
    Ok(())
}

fn active_revision_for_manifest(
    install_dir: &Path,
    manifest: &LocalModelManifest,
    preset: LocalEmbeddingPreset,
) -> Result<String> {
    let ref_relative = format!("{}/refs/main", preset.cache_repo_dir());
    if !manifest.files.iter().any(|file| file.path == ref_relative) {
        bail!("local embedding manifest is missing {ref_relative}");
    }
    read_active_revision(install_dir, &ref_relative)
}

fn read_active_revision(install_dir: &Path, relative_path: &str) -> Result<String> {
    let (path, _) = canonical_regular_path(install_dir, relative_path)?;
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let revision = content.trim();
    if !matches!(revision.len(), 40 | 64) || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!(
            "Hugging Face revision ref {} is not a 40- or 64-character hex revision",
            path.display()
        );
    }
    Ok(revision.to_string())
}

fn collect_snapshot_symlink(
    install_dir: &Path,
    path: &Path,
    preset: LocalEmbeddingPreset,
    revision: &str,
) -> Result<LocalModelSymlink> {
    let relative = relative_path_string(install_dir, path)?;
    let expected_prefix = format!("{}/snapshots/{revision}/", preset.cache_repo_dir());
    if !relative.starts_with(&expected_prefix) {
        bail!("model symlink is outside active snapshot: {relative}");
    }
    let link_target = std::fs::read_link(path)
        .with_context(|| format!("read symlink target {}", path.display()))?;
    let link_target = link_target.to_str().with_context(|| {
        format!(
            "local model symlink target is not Unicode: {}",
            path.display()
        )
    })?;
    let resolved = resolve_relative_symlink(install_dir, path, Path::new(link_target))?;
    let resolved_path = relative_path_string(install_dir, &resolved)?;
    let expected_blob_prefix = format!("{}/blobs/", preset.cache_repo_dir());
    if !resolved_path.starts_with(&expected_blob_prefix)
        || resolved_path[expected_blob_prefix.len()..].contains('/')
    {
        bail!("model symlink {relative} resolves outside repository blobs: {resolved_path}");
    }
    Ok(LocalModelSymlink {
        path: relative,
        link_target: link_target.to_string(),
        resolved_path,
    })
}

pub(super) fn resolve_relative_symlink(
    install_dir: &Path,
    link_path: &Path,
    target: &Path,
) -> Result<PathBuf> {
    if target.is_absolute() {
        bail!(
            "local model symlink {} has absolute target {}",
            link_path.display(),
            target.display()
        );
    }
    let parent = link_path
        .parent()
        .with_context(|| format!("symlink has no parent: {}", link_path.display()))?;
    let parent_relative = parent.strip_prefix(install_dir).with_context(|| {
        format!(
            "make {} relative to {}",
            parent.display(),
            install_dir.display()
        )
    })?;
    let mut components = parent_relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value.to_os_string()),
            _ => bail!(
                "unexpected model cache path component in {}",
                parent.display()
            ),
        })
        .collect::<Result<Vec<_>>>()?;
    for component in target.components() {
        match component {
            Component::Normal(value) => components.push(value.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                if components.pop().is_none() {
                    bail!(
                        "local model symlink {} escapes install directory via {}",
                        link_path.display(),
                        target.display()
                    );
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!(
                    "local model symlink {} has non-relative target {}",
                    link_path.display(),
                    target.display()
                )
            }
        }
    }
    let mut lexical = install_dir.to_path_buf();
    for component in components {
        lexical.push(component);
    }
    let metadata = std::fs::symlink_metadata(&lexical)
        .with_context(|| format!("stat symlink target {}", lexical.display()))?;
    if !metadata.file_type().is_file() {
        bail!(
            "local model symlink {} target is not a regular file: {}",
            link_path.display(),
            lexical.display()
        );
    }
    let canonical = std::fs::canonicalize(&lexical)
        .with_context(|| format!("canonicalize symlink target {}", lexical.display()))?;
    if canonical != lexical || !canonical.starts_with(install_dir) {
        bail!(
            "local model symlink {} resolves outside verified install directory: {}",
            link_path.display(),
            canonical.display()
        );
    }
    Ok(canonical)
}

fn insert_regular_file(
    install_dir: &Path,
    path: &Path,
    files: &mut HashMap<String, LocalModelFile>,
) -> Result<()> {
    let relative = relative_path_string(install_dir, path)?;
    let (canonical_path, metadata) = canonical_regular_path(install_dir, &relative)?;
    let sha256 = sha256_file(&canonical_path)?;
    let source_sha256 = source_sha256_from_hf_blob_path(&relative, &sha256)?;
    let file = LocalModelFile {
        path: relative.clone(),
        sha256,
        source_sha256,
        bytes: metadata.len(),
    };
    if let Some(existing) = files.insert(relative.clone(), file.clone()) {
        if existing != file {
            bail!("model artifact changed while collecting {relative}");
        }
    }
    Ok(())
}

pub(in crate::retrieval::embedding::local_semantic) fn canonical_regular_path(
    install_dir: &Path,
    relative_path: &str,
) -> Result<(PathBuf, std::fs::Metadata)> {
    let canonical_install_dir = std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize verified install {}", install_dir.display()))?;
    let lexical_path = canonical_install_dir.join(
        checked_relative_path(relative_path)
            .with_context(|| format!("validate regular path {relative_path}"))?,
    );
    let metadata = std::fs::symlink_metadata(&lexical_path)
        .with_context(|| format!("stat verified regular path {}", lexical_path.display()))?;
    if !metadata.file_type().is_file() {
        bail!(
            "verified path is not a regular file: {}",
            lexical_path.display()
        );
    }
    let canonical_path = std::fs::canonicalize(&lexical_path).with_context(|| {
        format!(
            "canonicalize verified regular path {}",
            lexical_path.display()
        )
    })?;
    if canonical_path != lexical_path || !canonical_path.starts_with(&canonical_install_dir) {
        bail!(
            "regular path {} canonical path {} escapes verified install {}",
            lexical_path.display(),
            canonical_path.display(),
            canonical_install_dir.display()
        );
    }
    Ok((canonical_path, metadata))
}

pub(super) fn relative_path_string(root: &Path, path: &Path) -> Result<String> {
    path.strip_prefix(root)
        .with_context(|| format!("make {} relative to {}", path.display(), root.display()))?
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_string)
                .with_context(|| format!("model cache path is not Unicode: {}", path.display())),
            _ => bail!("unexpected non-normal cache path {}", path.display()),
        })
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join("/"))
}

fn ensure_unique_paths<'a>(paths: impl Iterator<Item = &'a str>, kind: &str) -> Result<()> {
    let mut seen = HashSet::new();
    for path in paths {
        checked_relative_path(path)?;
        if !seen.insert(path) {
            bail!("duplicate local embedding manifest {kind} path: {path}");
        }
    }
    Ok(())
}
