use std::path::Path;

use anyhow::{bail, Context, Result};

use super::super::manifest::{
    collect_model_artifacts, model_content_sha256, verify_unpublished_manifest,
};
use super::super::{
    auto_artifact_is_trusted, LocalEmbeddingInputKind, LocalEmbeddingPreset, LocalModelManifest,
    TextEmbedding, AUTO_EVALUATED_DEFAULT_ARTIFACT_SHA256, FASTEMBED_RUNTIME,
    HUGGING_FACE_BASE_URL, MANIFEST_SCHEMA_VERSION,
};

#[derive(Debug)]
pub(in crate::retrieval::embedding::local_semantic) struct PreparedLocalModel {
    pub(in crate::retrieval::embedding::local_semantic) manifest: LocalModelManifest,
    pub(in crate::retrieval::embedding::local_semantic) artifact_sha256: String,
}

pub(in crate::retrieval::embedding::local_semantic) fn prepare_downloaded_model(
    preset: LocalEmbeddingPreset,
    staging_dir: &Path,
    downloaded_at_epoch: i64,
) -> Result<PreparedLocalModel> {
    prepare_downloaded_model_with(
        preset,
        staging_dir,
        downloaded_at_epoch,
        |manifest, artifact_sha256| {
            probe_verified_download(preset, staging_dir, manifest, artifact_sha256)
        },
    )
}

pub(in crate::retrieval::embedding::local_semantic) fn prepare_downloaded_model_with(
    preset: LocalEmbeddingPreset,
    staging_dir: &Path,
    downloaded_at_epoch: i64,
    probe: impl FnOnce(&LocalModelManifest, &str) -> Result<()>,
) -> Result<PreparedLocalModel> {
    let (files, symlinks) = collect_model_artifacts(staging_dir, preset)?;
    if files.is_empty() {
        bail!(
            "local embedding download did not materialize model files in {}",
            staging_dir.display()
        );
    }
    let manifest = LocalModelManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        preset: preset.label().to_string(),
        model_id: preset.model_id().to_string(),
        upstream_model: preset.upstream_model().to_string(),
        dimensions: preset.dimensions(),
        runtime: FASTEMBED_RUNTIME.to_string(),
        source_url: Some(preset.source_url()),
        downloaded_at_epoch,
        files,
        symlinks,
    };
    let artifact_sha256 = model_content_sha256(&manifest)?;
    if preset == LocalEmbeddingPreset::default()
        && !auto_artifact_is_trusted(staging_dir, &artifact_sha256)?
    {
        bail!(
            "downloaded default local embedding model {} has unapproved content sha256:{artifact_sha256}; expected evaluated sha256:{}. No downloaded model bytes were executed. Upgrade remem or retry after {} publishes the evaluated artifact",
            preset.label(),
            AUTO_EVALUATED_DEFAULT_ARTIFACT_SHA256,
            HUGGING_FACE_BASE_URL
        );
    }
    let verified_artifact_sha256 =
        verify_unpublished_manifest(staging_dir, &manifest, Some(preset))?;
    if verified_artifact_sha256 != artifact_sha256 {
        bail!(
            "local embedding content identity changed while preparing {}",
            preset.label()
        );
    }
    probe(&manifest, &artifact_sha256)?;
    Ok(PreparedLocalModel {
        manifest,
        artifact_sha256,
    })
}

fn probe_verified_download(
    preset: LocalEmbeddingPreset,
    staging_dir: &Path,
    manifest: &LocalModelManifest,
    artifact_sha256: &str,
) -> Result<()> {
    let values = super::super::runtime::probe_verified_model(
        preset,
        staging_dir,
        manifest,
        artifact_sha256,
        "remem local embedding readiness probe",
        LocalEmbeddingInputKind::Generic,
    )
    .with_context(|| format!("probe verified local embedding model {}", preset.label()))?;
    if values.len() != preset.dimensions() {
        bail!(
            "local embedding model {} returned {} probe dimensions, expected {}",
            preset.label(),
            values.len(),
            preset.dimensions()
        );
    }
    TextEmbedding::new(
        format!("{}@sha256:{artifact_sha256}", preset.model_id()),
        values,
    )
    .with_context(|| format!("validate probe embedding from {}", preset.label()))?;
    Ok(())
}
