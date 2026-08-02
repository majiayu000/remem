use std::path::Path;

use anyhow::{bail, Context, Result};
use hf_hub::api::sync::{Api, ApiBuilder};
use hf_hub::{Cache, Repo, RepoType};

use super::{LocalEmbeddingPreset, HUGGING_FACE_BASE_URL, HUGGING_FACE_ENDPOINT_ENV};

const EVALUATED_E5_HUGGING_FACE_REVISION: &str = "614241f622f53c4eeff9890bdc4f31cfecc418b3";

mod prepare;
mod publish;
mod staging;
#[cfg(test)]
mod tests;

pub(super) use prepare::prepare_downloaded_model;
#[cfg(test)]
pub(super) use prepare::prepare_downloaded_model_with;
#[cfg(test)]
pub(super) use publish::ImportedLocalModel;
pub(super) use publish::{activate_candidate_manifest, import_immutable_candidate};
pub(super) use staging::DownloadStaging;

pub(super) fn materialize_hugging_face_artifacts(
    preset: LocalEmbeddingPreset,
    install_dir: &Path,
) -> Result<DownloadStaging> {
    materialize_hugging_face_artifacts_with(install_dir, |staging_dir| {
        download_required_runtime_files(preset, staging_dir)
    })
}

pub(super) fn materialize_hugging_face_artifacts_with(
    install_dir: &Path,
    download: impl FnOnce(&Path) -> Result<()>,
) -> Result<DownloadStaging> {
    with_official_hugging_face_endpoint(|| {
        let staging = DownloadStaging::create(install_dir)?;
        match download(staging.path()) {
            Ok(()) => Ok(staging),
            Err(error) => cleanup_staging_after_error(staging, error),
        }
    })
}

pub(super) fn cleanup_staging_after_error<T>(
    staging: DownloadStaging,
    error: anyhow::Error,
) -> Result<T> {
    match staging.cleanup() {
        Ok(()) => Err(error),
        Err(cleanup_error) => Err(anyhow::anyhow!(
            "{error:#}; additionally failed to remove local model download staging: {cleanup_error:#}"
        )),
    }
}

pub(super) fn with_official_hugging_face_endpoint<T>(
    download: impl FnOnce() -> Result<T>,
) -> Result<T> {
    ensure_official_hugging_face_endpoint()?;
    download()
}

fn download_required_runtime_files(preset: LocalEmbeddingPreset, install_dir: &Path) -> Result<()> {
    let install_metadata = std::fs::symlink_metadata(install_dir)
        .with_context(|| format!("stat local model cache {}", install_dir.display()))?;
    if install_metadata.file_type().is_symlink() || !install_metadata.file_type().is_dir() {
        bail!(
            "local model cache must be a real directory, not a symlink or special file: {}",
            install_dir.display()
        );
    }
    let canonical_install_dir = std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize local model cache {}", install_dir.display()))?;
    let (pinned_repo, revision) = approved_download_repo(preset)?;
    let api = build_unauthenticated_download_api(&canonical_install_dir, HUGGING_FACE_BASE_URL)?;
    let pinned_api_repo = api.repo(pinned_repo);

    for runtime_file in preset.required_runtime_files() {
        let path = pinned_api_repo.get(runtime_file).with_context(|| {
            format!(
                "download verified-input file {runtime_file} for {} at revision {revision}",
                preset.upstream_model()
            )
        })?;
        validate_downloaded_pointer(
            &canonical_install_dir,
            preset,
            &revision,
            runtime_file,
            &path,
        )?;
    }

    let pinned_ref = canonical_install_dir
        .join(preset.cache_repo_dir())
        .join("refs")
        .join(&revision);
    let resolved_revision = read_revision_ref(&pinned_ref)?;
    if resolved_revision != revision {
        bail!(
            "Hugging Face returned revision {resolved_revision} while downloading pinned revision {revision} for {}",
            preset.upstream_model()
        );
    }
    validate_main_ref_destination(&canonical_install_dir, preset)?;
    Cache::new(canonical_install_dir)
        .repo(Repo::new(
            preset.upstream_model().to_string(),
            RepoType::Model,
        ))
        .create_ref(&revision)
        .with_context(|| {
            format!(
                "publish active Hugging Face revision {revision} for {}",
                preset.upstream_model()
            )
        })
}

fn approved_download_repo(preset: LocalEmbeddingPreset) -> Result<(Repo, String)> {
    let approved_revision = match preset {
        LocalEmbeddingPreset::MultilingualE5Small => EVALUATED_E5_HUGGING_FACE_REVISION,
        _ => bail!(
            "automatic download for local embedding preset {} is unavailable because it has no approved immutable Hugging Face revision; use multilingual-e5-small or continue using an already installed verified {} cache",
            preset.label(),
            preset.label(),
        ),
    };
    let revision = validate_revision(approved_revision)?;
    Ok((
        Repo::with_revision(
            preset.upstream_model().to_string(),
            RepoType::Model,
            revision.clone(),
        ),
        revision,
    ))
}

fn build_unauthenticated_download_api(cache_dir: &Path, endpoint: &str) -> Result<Api> {
    // `ApiBuilder::from_cache` reads a token from the cache's parent before
    // callers can clear it. Bootstrap it from a nonexistent directory inside
    // this freshly created staging cache, then switch to the real cache and
    // explicitly disable authorization.
    let tokenless_bootstrap = Cache::new(cache_dir.join(".remem-tokenless-bootstrap").join("hub"));
    ApiBuilder::from_cache(tokenless_bootstrap)
        .with_cache_dir(cache_dir.to_path_buf())
        .with_token(None)
        .with_endpoint(endpoint.to_string())
        .with_progress(true)
        .build()
        .context("initialize official Hugging Face download client")
}

fn validate_downloaded_pointer(
    install_dir: &Path,
    preset: LocalEmbeddingPreset,
    revision: &str,
    runtime_file: &str,
    path: &Path,
) -> Result<()> {
    let expected = install_dir
        .join(preset.cache_repo_dir())
        .join("snapshots")
        .join(revision)
        .join(runtime_file);
    if path != expected {
        bail!(
            "Hugging Face cache returned unexpected path {} for {runtime_file}; expected {}",
            path.display(),
            expected.display()
        );
    }
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("stat {}", path.display()))?;
    if !metadata.file_type().is_file() && !metadata.file_type().is_symlink() {
        bail!(
            "downloaded Hugging Face runtime path is not a file or symlink: {}",
            path.display()
        );
    }
    let resolved =
        std::fs::canonicalize(path).with_context(|| format!("resolve {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        let blob_dir = install_dir.join(preset.cache_repo_dir()).join("blobs");
        let blob_dir = std::fs::canonicalize(&blob_dir).with_context(|| {
            format!("canonicalize Hugging Face blob dir {}", blob_dir.display())
        })?;
        if resolved.parent() != Some(blob_dir.as_path()) {
            bail!(
                "downloaded Hugging Face pointer {} resolves outside verified blob directory {}",
                path.display(),
                blob_dir.display()
            );
        }
    } else if resolved != expected {
        bail!(
            "downloaded Hugging Face file resolved unexpectedly: {} -> {}",
            path.display(),
            resolved.display()
        );
    }
    let resolved_metadata =
        std::fs::metadata(&resolved).with_context(|| format!("stat {}", resolved.display()))?;
    if !resolved_metadata.is_file() {
        bail!(
            "downloaded Hugging Face artifact is not a regular file: {}",
            resolved.display()
        );
    }
    Ok(())
}

fn validate_main_ref_destination(install_dir: &Path, preset: LocalEmbeddingPreset) -> Result<()> {
    let repo_dir = install_dir.join(preset.cache_repo_dir());
    let refs_dir = repo_dir.join("refs");
    let refs_metadata = std::fs::symlink_metadata(&refs_dir)
        .with_context(|| format!("stat Hugging Face refs dir {}", refs_dir.display()))?;
    if refs_metadata.file_type().is_symlink() || !refs_metadata.file_type().is_dir() {
        bail!(
            "Hugging Face refs path is not a real directory: {}",
            refs_dir.display()
        );
    }
    let main_ref = refs_dir.join("main");
    match std::fs::symlink_metadata(&main_ref) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => bail!(
            "refusing to publish Hugging Face main revision through non-file {}",
            main_ref.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("stat Hugging Face main ref {}", main_ref.display()))
        }
    }
}

fn read_revision_ref(path: &Path) -> Result<String> {
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("stat {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        bail!(
            "Hugging Face revision ref is not a real file: {}",
            path.display()
        );
    }
    let revision =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    validate_revision(revision.trim())
}

fn validate_revision(raw: &str) -> Result<String> {
    let revision = raw.trim();
    if !matches!(revision.len(), 40 | 64) || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!(
            "Hugging Face revision must be a 40- or 64-character hexadecimal commit, got {revision}"
        );
    }
    Ok(revision.to_string())
}

fn ensure_official_hugging_face_endpoint() -> Result<()> {
    let Some(endpoint) = std::env::var_os(HUGGING_FACE_ENDPOINT_ENV) else {
        return Ok(());
    };
    let endpoint = endpoint.into_string().map_err(|_| {
        anyhow::anyhow!(
            "{HUGGING_FACE_ENDPOINT_ENV} contains non-Unicode data; unset it or set it to {HUGGING_FACE_BASE_URL}"
        )
    })?;
    let normalized = endpoint.strip_suffix('/').unwrap_or(&endpoint);
    if endpoint.is_empty() || normalized != HUGGING_FACE_BASE_URL {
        bail!(
            "{HUGGING_FACE_ENDPOINT_ENV} must be unset or exactly {HUGGING_FACE_BASE_URL}; got {}",
            if endpoint.is_empty() {
                "<empty>"
            } else {
                endpoint.as_str()
            }
        );
    }
    Ok(())
}
