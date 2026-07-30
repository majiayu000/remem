use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use fastembed::{OutputKey, Pooling, SingleBatchOutput, TokenizerFiles};
use ndarray::Array;
use ort::{
    session::{
        builder::{GraphOptimizationLevel, SessionBuilder},
        Session,
    },
    value::Value,
};
use tokenizers::{AddedToken, PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

use super::super::{checked_relative_path, LocalEmbeddingPreset, TOKENIZER_RUNTIME_FILES};

const DEFAULT_OUTPUT_PRECEDENCE: &[OutputKey] = &[
    OutputKey::OnlyOne,
    OutputKey::ByName("text_embeds"),
    OutputKey::ByName("last_hidden_state"),
    OutputKey::ByName("sentence_embedding"),
];

pub(super) struct FileBackedTextEmbedding {
    tokenizer: Tokenizer,
    session: Session,
    need_token_type_ids: bool,
    pooling: Option<Pooling>,
    output_key: Option<OutputKey>,
}

impl FileBackedTextEmbedding {
    pub(super) fn load(
        preset: LocalEmbeddingPreset,
        artifact_sha256: &str,
        private_root: &Path,
    ) -> Result<Self> {
        if preset != LocalEmbeddingPreset::BgeM3 {
            bail!(
                "file-backed local embedding loader does not support {}",
                preset.label()
            );
        }

        let fastembed_model = preset.fastembed_model();
        let output_key = fastembed::TextEmbedding::get_model_info(&fastembed_model)?
            .output_key
            .clone();
        let pooling = fastembed::TextEmbedding::get_default_pooling_method(&fastembed_model);
        let options: fastembed::InitOptionsUserDefined =
            fastembed::TextInitOptions::new(fastembed_model).into();
        let max_length = options.max_length;
        let model_path =
            private_runtime_file(private_root, preset, artifact_sha256, preset.model_file())?;
        let session = load_session_from_file(&model_path, options)?;
        let tokenizer_files = load_tokenizer_files(preset, artifact_sha256, private_root)?;
        let tokenizer = load_tokenizer(tokenizer_files, max_length)?;
        let need_token_type_ids = session
            .inputs()
            .iter()
            .any(|input| input.name() == "token_type_ids");

        Ok(Self {
            tokenizer,
            session,
            need_token_type_ids,
            pooling,
            output_key,
        })
    }

    pub(super) fn embed(&mut self, input: &str) -> Result<Vec<f32>> {
        let encodings = self
            .tokenizer
            .encode_batch(vec![input], true)
            .map_err(|error| anyhow::Error::msg(error.to_string()))
            .context("encode file-backed local embedding input")?;
        let encoding_length = encodings
            .first()
            .context("tokenizer returned empty encodings")?
            .len();
        let batch_size = encodings.len();
        let max_size = encoding_length * batch_size;
        let mut ids = Vec::with_capacity(max_size);
        let mut masks = Vec::with_capacity(max_size);
        let mut type_ids = Vec::with_capacity(max_size);

        for encoding in &encodings {
            ids.extend(encoding.get_ids().iter().map(|value| i64::from(*value)));
            masks.extend(
                encoding
                    .get_attention_mask()
                    .iter()
                    .map(|value| i64::from(*value)),
            );
            type_ids.extend(
                encoding
                    .get_type_ids()
                    .iter()
                    .map(|value| i64::from(*value)),
            );
        }

        let input_ids = Array::from_shape_vec((batch_size, encoding_length), ids)?;
        let attention_mask = Array::from_shape_vec((batch_size, encoding_length), masks)?;
        let token_type_ids = Array::from_shape_vec((batch_size, encoding_length), type_ids)?;
        let mut session_inputs = ort::inputs![
            "input_ids" => Value::from_array(input_ids)?,
            "attention_mask" => Value::from_array(attention_mask.clone())?,
        ];
        if self.need_token_type_ids {
            session_inputs.push((
                "token_type_ids".into(),
                Value::from_array(token_type_ids)?.into(),
            ));
        }

        let outputs = self
            .session
            .run(session_inputs)
            .map_err(anyhow::Error::new)?
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect();
        let batch_output = SingleBatchOutput {
            outputs,
            attention_mask_array: attention_mask,
        };
        let pooled = match &self.output_key {
            Some(output_key) => {
                batch_output.select_and_pool_output(&output_key, self.pooling.clone())?
            }
            None => batch_output
                .select_and_pool_output(&DEFAULT_OUTPUT_PRECEDENCE, self.pooling.clone())?,
        };
        let mut rows = pooled.rows().into_iter();
        let row = rows
            .next()
            .context("file-backed local embedding model returned no embedding")?;
        if rows.next().is_some() {
            bail!("file-backed local embedding model returned multiple embeddings");
        }
        let values = row
            .as_slice()
            .context("file-backed local embedding row is not contiguous")?;
        Ok(normalize_fastembed(values))
    }
}

fn load_session_from_file(
    model_path: &Path,
    options: fastembed::InitOptionsUserDefined,
) -> Result<Session> {
    let threads = options
        .intra_threads
        .map(Ok)
        .unwrap_or_else(|| std::thread::available_parallelism().map(usize::from))?;
    let builder_error = |error: ort::Error<SessionBuilder>| anyhow::Error::msg(error.to_string());
    let mut builder = Session::builder()?
        .with_execution_providers(options.execution_providers)
        .map_err(builder_error)?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(builder_error)?
        .with_intra_threads(threads)
        .map_err(builder_error)?;
    builder
        .commit_from_file(model_path)
        .with_context(|| format!("open verified ONNX model {}", model_path.display()))
}

fn load_tokenizer_files(
    preset: LocalEmbeddingPreset,
    artifact_sha256: &str,
    private_root: &Path,
) -> Result<TokenizerFiles> {
    Ok(TokenizerFiles {
        tokenizer_file: read_private_runtime_bytes(
            preset,
            artifact_sha256,
            private_root,
            TOKENIZER_RUNTIME_FILES[0],
        )?,
        config_file: read_private_runtime_bytes(
            preset,
            artifact_sha256,
            private_root,
            TOKENIZER_RUNTIME_FILES[1],
        )?,
        special_tokens_map_file: read_private_runtime_bytes(
            preset,
            artifact_sha256,
            private_root,
            TOKENIZER_RUNTIME_FILES[2],
        )?,
        tokenizer_config_file: read_private_runtime_bytes(
            preset,
            artifact_sha256,
            private_root,
            TOKENIZER_RUNTIME_FILES[3],
        )?,
    })
}

fn read_private_runtime_bytes(
    preset: LocalEmbeddingPreset,
    artifact_sha256: &str,
    private_root: &Path,
    runtime_file: &str,
) -> Result<Vec<u8>> {
    let path = private_runtime_file(private_root, preset, artifact_sha256, runtime_file)?;
    std::fs::read(&path)
        .with_context(|| format!("read private runtime tokenizer file {}", path.display()))
}

fn private_runtime_file(
    private_root: &Path,
    preset: LocalEmbeddingPreset,
    artifact_sha256: &str,
    runtime_file: &str,
) -> Result<PathBuf> {
    Ok(private_root
        .join(preset.cache_repo_dir())
        .join("snapshots")
        .join(artifact_sha256)
        .join(checked_relative_path(runtime_file)?))
}

// Keep these semantics aligned with fastembed::common::load_tokenizer. The helper is private in
// fastembed, so a file-backed model cannot reuse it without routing model loading through hf-hub.
fn load_tokenizer(tokenizer_files: TokenizerFiles, max_length: usize) -> Result<Tokenizer> {
    let invalid_file = |name: &str| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "Error building TokenizerFiles for UserDefinedEmbeddingModel. Could not read {name} file."
            ),
        )
    };
    let config: serde_json::Value = serde_json::from_slice(&tokenizer_files.config_file)
        .map_err(|_| invalid_file("config.json"))?;
    let special_tokens_map: serde_json::Value =
        serde_json::from_slice(&tokenizer_files.special_tokens_map_file)
            .map_err(|_| invalid_file("special_tokens_map.json"))?;
    let tokenizer_config: serde_json::Value =
        serde_json::from_slice(&tokenizer_files.tokenizer_config_file)
            .map_err(|_| invalid_file("tokenizer_config.json"))?;
    let mut tokenizer = Tokenizer::from_bytes(tokenizer_files.tokenizer_file)
        .map_err(|_| invalid_file("tokenizer.json"))?;

    let model_max_length = tokenizer_config["model_max_length"]
        .as_f64()
        .context("tokenizer_config.json is missing a numeric `model_max_length` field")?
        as f32;
    let max_length = max_length.min(model_max_length as usize);
    let pad_id = config["pad_token_id"].as_u64().unwrap_or(0) as u32;
    let pad_token = tokenizer_config["pad_token"]
        .as_str()
        .context("tokenizer_config.json is missing a string `pad_token` field")?
        .to_string();
    let mut tokenizer = tokenizer
        .with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            pad_token,
            pad_id,
            ..Default::default()
        }))
        .with_truncation(Some(TruncationParams {
            max_length,
            ..Default::default()
        }))
        .map_err(anyhow::Error::msg)?
        .clone();

    if let serde_json::Value::Object(root_object) = special_tokens_map {
        for value in root_object.values() {
            if let Some(content) = value.as_str() {
                tokenizer.add_special_tokens(&[AddedToken {
                    content: content.to_string(),
                    special: true,
                    ..Default::default()
                }]);
            } else if let (
                Some(content),
                Some(single_word),
                Some(lstrip),
                Some(rstrip),
                Some(normalized),
            ) = (
                value["content"].as_str(),
                value["single_word"].as_bool(),
                value["lstrip"].as_bool(),
                value["rstrip"].as_bool(),
                value["normalized"].as_bool(),
            ) {
                tokenizer.add_special_tokens(&[AddedToken {
                    content: content.to_string(),
                    special: true,
                    single_word,
                    lstrip,
                    rstrip,
                    normalized,
                }]);
            }
        }
    }
    Ok(tokenizer.into())
}

fn normalize_fastembed(values: &[f32]) -> Vec<f32> {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    let epsilon = 1e-12;
    values
        .iter()
        .map(|value| value / (norm + epsilon))
        .collect()
}
