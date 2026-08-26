use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use super::types::{
    BenchVerifyFailure, BenchVerifyOptions, BenchVerifyReport, BenchmarkLayer, MemoryRunArtifact,
    PublicBenchmarkManifest, PublicBenchmarkReport,
};

pub(super) mod coding;

const REQUIRED_SCHEMA_FILES: [&str; 6] = [
    "schemas/benchmark-manifest.schema.json",
    "schemas/memory-run.schema.json",
    "schemas/coding-run.schema.json",
    "schemas/memory-report.schema.json",
    "schemas/coding-report.schema.json",
    "schemas/reproduction-metadata.schema.json",
];

const MEMORY_ARTIFACT_KEYS: [&str; 5] = [
    "reader_input",
    "retrieved_evidence",
    "answer",
    "score",
    "diagnosis",
];

pub fn verify_benchmark_artifacts(options: BenchVerifyOptions) -> Result<BenchVerifyReport> {
    let root = options.root;
    let mut state = VerifyState::new(root.clone());

    if !root.exists() {
        state.fail(".".to_string(), "benchmark root does not exist");
        return Ok(state.finish());
    }
    if !root.is_dir() {
        state.fail(".".to_string(), "benchmark root is not a directory");
        return Ok(state.finish());
    }

    validate_required_schemas(&mut state);

    let manifest_paths = collect_manifest_paths(&root)?;
    if manifest_paths.is_empty() {
        state.fail(
            rel_display(&root, &root),
            "benchmark root has no manifests under a manifests/ directory",
        );
    }

    for manifest_path in manifest_paths {
        state.manifests_checked += 1;
        let Some(manifest) =
            read_json::<PublicBenchmarkManifest>(&manifest_path, &mut state, "manifest")
        else {
            continue;
        };
        validate_manifest(&manifest_path, &manifest, &mut state);
        for report_path in &manifest.reports {
            let Some(report_abs) = resolve_public_path(&mut state, report_path, report_path) else {
                continue;
            };
            validate_report_path_layer(&manifest_path, &manifest, &report_abs, &mut state);
        }
    }

    Ok(state.finish())
}

fn validate_required_schemas(state: &mut VerifyState) {
    for relative in REQUIRED_SCHEMA_FILES {
        let Some(path) = resolve_public_path(state, relative, relative) else {
            continue;
        };
        if !path.exists() {
            state.fail(relative.to_string(), "required schema file is missing");
            continue;
        }
        let Some(value) = read_json::<Value>(&path, state, "schema") else {
            continue;
        };
        if value.get("$schema").and_then(Value::as_str).is_none() {
            state.fail(relative.to_string(), "schema is missing $schema");
        }
        if value
            .pointer("/properties/schema_version/const")
            .and_then(Value::as_u64)
            != Some(1)
        {
            state.fail(
                relative.to_string(),
                "schema must pin schema_version const 1",
            );
        }
    }
}

fn validate_manifest(path: &Path, manifest: &PublicBenchmarkManifest, state: &mut VerifyState) {
    let label = rel_display(&state.root, path);
    if manifest.schema_version != 1 {
        state.fail(label.clone(), "manifest schema_version must be 1");
    }
    require_non_blank(&manifest.benchmark_id, &label, "benchmark_id", state);
    require_non_blank(&manifest.version, &label, "version", state);
    if manifest.created_at_epoch <= 0 {
        state.fail(label.clone(), "manifest created_at_epoch must be positive");
    }
    if manifest.conditions.is_empty() {
        state.fail(label.clone(), "manifest conditions must not be empty");
    }
    validate_condition_list(
        &manifest.conditions,
        manifest.layer,
        &label,
        "manifest conditions",
        state,
    );
    if manifest.reports.is_empty() {
        state.fail(label.clone(), "manifest reports must not be empty");
    }
    if manifest.source_policy.private_user_memory_allowed {
        state.fail(
            label.clone(),
            "public benchmark manifest must not allow private user memory",
        );
    }
    if !manifest.source_policy.requires_temp_remem_data_dir {
        state.fail(
            label,
            "public benchmark manifest must require temporary REMEM_DATA_DIR isolation",
        );
    }
    if let Some(revision) = manifest.source_policy.external_dataset_revision.as_deref() {
        scan_private_string(
            revision,
            path,
            "source_policy.external_dataset_revision",
            state,
        );
    }
}

fn validate_report_path_layer(
    manifest_path: &Path,
    manifest: &PublicBenchmarkManifest,
    report_path: &Path,
    state: &mut VerifyState,
) {
    state.reports_checked += 1;
    let Some(report) = read_json::<PublicBenchmarkReport>(report_path, state, "report") else {
        return;
    };
    let label = rel_display(&state.root, report_path);
    if report.schema_version != 1 {
        state.fail(label.clone(), "report schema_version must be 1");
    }
    require_non_blank(&report.benchmark_id, &label, "benchmark_id", state);
    require_non_blank(
        &report.benchmark_version,
        &label,
        "benchmark_version",
        state,
    );
    if report.layer == BenchmarkLayer::CodingAgentOutcome {
        match report.run_phase.as_deref() {
            Some(run_phase) => require_non_blank(run_phase, &label, "run_phase", state),
            None => state.fail(label.clone(), "coding report run_phase is missing"),
        }
        match report.matrix_namespace.as_deref() {
            Some(namespace) => require_non_blank(namespace, &label, "matrix_namespace", state),
            None => state.fail(label.clone(), "coding report matrix_namespace is missing"),
        }
    }
    require_non_blank(&report.claim_level, &label, "claim_level", state);
    if report.layer != manifest.layer {
        state.fail(
            label.clone(),
            "report layer must match the manifest layer that references it",
        );
    }
    if report.benchmark_id != manifest.benchmark_id {
        state.fail(
            label.clone(),
            "report benchmark_id must match the manifest benchmark_id",
        );
    }
    if report.benchmark_version != manifest.version {
        state.fail(
            label.clone(),
            format!(
                "report benchmark_version {:?} must match manifest version {:?}",
                report.benchmark_version, manifest.version
            ),
        );
    }
    if report.conditions.is_empty() {
        state.fail(label.clone(), "report conditions must not be empty");
    }
    validate_condition_list(
        &report.conditions,
        report.layer,
        &label,
        "report conditions",
        state,
    );
    if condition_set(&report.conditions) != condition_set(&manifest.conditions) {
        state.fail(
            label.clone(),
            "report conditions must exactly match manifest conditions",
        );
    }
    if report.schema_refs.is_empty() {
        state.fail(label.clone(), "report schema_refs must not be empty");
    }
    if report.run_artifacts.is_empty() {
        state.fail(label.clone(), "report run_artifacts must not be empty");
    }
    if !report.verifier.required || report.verifier.schema_version != 1 {
        state.fail(
            label.clone(),
            "report verifier metadata must require schema_version 1",
        );
    }
    if report.aggregate_metrics.is_null() {
        state.fail(label.clone(), "report aggregate_metrics must be present");
    }
    scan_private_json(
        &serde_json::to_value(&report).unwrap_or(Value::Null),
        report_path,
        "$",
        state,
    );
    for schema_ref in &report.schema_refs {
        let Some(schema_path) = resolve_public_path(state, schema_ref, schema_ref) else {
            continue;
        };
        if !schema_path.exists() {
            state.fail(schema_ref.clone(), "report schema_ref does not exist");
        }
    }
    let mut represented_conditions = BTreeSet::new();
    for run_artifact in &report.run_artifacts {
        let Some(run_path) = resolve_public_path(state, run_artifact, run_artifact) else {
            continue;
        };
        let run_label = rel_display(&state.root, &run_path);
        if !state.run_artifact_paths.insert(run_label.clone()) {
            state.fail(run_label, "run artifact path must be referenced only once");
            continue;
        }
        let represented_condition = match report.layer {
            BenchmarkLayer::MemorySystemCapability => {
                validate_memory_run_artifact(&run_path, &report, manifest_path, state)
            }
            BenchmarkLayer::CodingAgentOutcome => {
                coding::validate_coding_run_artifact(&run_path, &report, manifest_path, state)
            }
        };
        if let Some(condition) = represented_condition {
            represented_conditions.insert(condition);
        }
    }
    if represented_conditions != condition_set(&report.conditions) {
        state.fail(
            label,
            "report conditions must exactly match conditions represented by run artifacts",
        );
    }
}

fn validate_memory_run_artifact(
    run_path: &Path,
    report: &PublicBenchmarkReport,
    _manifest_path: &Path,
    state: &mut VerifyState,
) -> Option<String> {
    state.run_artifacts_checked += 1;
    let run = read_json::<MemoryRunArtifact>(run_path, state, "memory run artifact")?;
    let label = rel_display(&state.root, run_path);
    if run.schema_version != 1 {
        state.fail(label.clone(), "memory run schema_version must be 1");
    }
    if run.layer != BenchmarkLayer::MemorySystemCapability || run.layer != report.layer {
        state.fail(
            label.clone(),
            "memory run layer must be memory_system_capability",
        );
    }
    require_non_blank(&run.benchmark_id, &label, "benchmark_id", state);
    if run.benchmark_id != report.benchmark_id {
        state.fail(
            label.clone(),
            format!(
                "memory run benchmark_id {:?} must match report benchmark_id {:?}",
                run.benchmark_id, report.benchmark_id
            ),
        );
    }
    require_non_blank(&run.benchmark_version, &label, "benchmark_version", state);
    if run.benchmark_version != report.benchmark_version {
        state.fail(
            label.clone(),
            format!(
                "memory run benchmark_version {:?} must match report benchmark_version {:?}",
                run.benchmark_version, report.benchmark_version
            ),
        );
    }
    require_non_blank(&run.suite, &label, "suite", state);
    match report.suite.as_deref() {
        Some(report_suite) => {
            require_non_blank(report_suite, &label, "report suite", state);
            if run.suite != report_suite {
                state.fail(
                    label.clone(),
                    format!(
                        "memory run suite {:?} must match report suite {:?}",
                        run.suite, report_suite
                    ),
                );
            }
        }
        None => state.fail(label.clone(), "memory report suite is missing"),
    }
    require_non_blank(&run.condition, &label, "condition", state);
    if !report.conditions.contains(&run.condition) {
        state.fail(
            label.clone(),
            "memory run condition must be declared by its report",
        );
    }
    require_non_blank(&run.task_id, &label, "task_id", state);
    if run.reference_time_epoch <= 0 {
        state.fail(
            label.clone(),
            "memory run reference_time_epoch must be positive",
        );
    }
    validate_environment(&run.environment, &label, state);
    if run.reader_model.is_null() {
        state.fail(label.clone(), "memory run reader_model must be present");
    }
    if run.answer.is_null() {
        state.fail(label.clone(), "memory run answer must be present");
    }
    if run.metrics.is_null() {
        state.fail(label.clone(), "memory run metrics must be present");
    }
    if !run.diagnosis.write_side_gap
        && !run.diagnosis.retrieval_side_gap
        && !run.diagnosis.reader_gap
        && !run.diagnosis.policy_abstention
        && run
            .diagnosis
            .notes
            .iter()
            .any(|note| note.trim().is_empty())
    {
        state.fail(
            label.clone(),
            "memory run diagnosis notes must not be blank",
        );
    }
    let abstained = run
        .answer
        .get("abstained")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if run.retrieval.gold_supporting_event_ids.is_empty() {
        state.fail(
            label.clone(),
            "memory run is missing gold supporting evidence IDs",
        );
    }
    if !abstained && run.condition != "no_memory" && run.retrieval.retrieved_memory_ids.is_empty() {
        state.fail(
            label.clone(),
            "memory run condition requires retrieved memory IDs",
        );
    }
    let diagnosis_explains_abstention = run.diagnosis.policy_abstention
        || run.diagnosis.write_side_gap
        || run.diagnosis.retrieval_side_gap
        || run.diagnosis.reader_gap
        || run.condition == "no_memory";
    if abstained && !diagnosis_explains_abstention {
        state.fail(
            label.clone(),
            "abstained memory run must mark a diagnosis reason",
        );
    }
    if !abstained {
        if run.condition != "no_memory" && run.evidence.cited_memory_ids.is_empty() {
            state.fail(label.clone(), "memory run is missing cited memory IDs");
        }
        if run.evidence.cited_event_ids.is_empty() {
            state.fail(label.clone(), "memory run is missing cited event IDs");
        }
    }
    for id in &run.retrieval.retrieved_supporting_evidence_ids {
        require_non_blank(id, &label, "retrieved_supporting_evidence_ids", state);
    }
    for id in &run.retrieval.missing_supporting_evidence_ids {
        require_non_blank(id, &label, "missing_supporting_evidence_ids", state);
    }
    validate_artifact_map(&run.artifacts, MEMORY_ARTIFACT_KEYS, &label, state);
    scan_private_json(
        &serde_json::to_value(&run).unwrap_or(Value::Null),
        run_path,
        "$",
        state,
    );
    Some(run.condition)
}

fn validate_environment(env: &super::types::RunEnvironment, label: &str, state: &mut VerifyState) {
    require_non_blank(&env.os, label, "environment.os", state);
    require_non_blank(&env.arch, label, "environment.arch", state);
    require_non_blank(&env.remem_commit, label, "environment.remem_commit", state);
    require_non_blank(
        &env.remem_data_dir,
        label,
        "environment.remem_data_dir",
        state,
    );
    if !env.remem_data_dir.starts_with("temp://")
        && !env.remem_data_dir.starts_with("/tmp/")
        && !env.remem_data_dir.starts_with("/private/tmp/")
    {
        state.fail(
            label.to_string(),
            "environment.remem_data_dir must prove temporary isolation",
        );
    }
    if let Some(digest) = env.docker_image_digest.as_deref() {
        require_non_blank(digest, label, "environment.docker_image_digest", state);
    }
    if let Some(revision) = env.fixture_revision.as_deref() {
        require_non_blank(revision, label, "environment.fixture_revision", state);
    }
    if let Some(commit) = env.repo_base_commit.as_deref() {
        require_non_blank(commit, label, "environment.repo_base_commit", state);
    }
}

fn validate_artifact_map<const N: usize>(
    artifacts: &std::collections::BTreeMap<String, String>,
    required_keys: [&str; N],
    label: &str,
    state: &mut VerifyState,
) {
    for key in required_keys {
        require_artifact_key(artifacts, key, label, state);
    }
}

fn require_artifact_key(
    artifacts: &std::collections::BTreeMap<String, String>,
    key: &str,
    label: &str,
    state: &mut VerifyState,
) {
    let Some(raw_path) = artifacts.get(key) else {
        state.fail(label.to_string(), format!("artifact key {key} is missing"));
        return;
    };
    let Some(path) = resolve_public_path(state, raw_path, raw_path) else {
        return;
    };
    if !path.exists() {
        state.fail(
            raw_path.clone(),
            format!("artifact file for {key} is missing"),
        );
        return;
    }
    state.artifact_files.insert(rel_display(&state.root, &path));
}

fn require_non_blank(value: &str, label: &str, field: &str, state: &mut VerifyState) {
    if value.trim().is_empty() {
        state.fail(label.to_string(), format!("{field} must not be blank"));
    }
}

fn validate_condition_list(
    conditions: &[String],
    layer: BenchmarkLayer,
    label: &str,
    field: &str,
    state: &mut VerifyState,
) {
    let mut seen = BTreeSet::new();
    for condition in conditions {
        require_non_blank(condition, label, field, state);
        if !seen.insert(condition.as_str()) {
            state.fail(label.to_string(), format!("{field} must be unique"));
        }
        if layer == BenchmarkLayer::CodingAgentOutcome
            && !coding::is_public_coding_condition(condition)
        {
            state.fail(
                label.to_string(),
                format!("{field} contains unknown coding condition identity {condition}"),
            );
        }
    }
}

fn condition_set(conditions: &[String]) -> BTreeSet<String> {
    conditions.iter().cloned().collect()
}

fn read_json<T: DeserializeOwned>(path: &Path, state: &mut VerifyState, label: &str) -> Option<T> {
    let display = rel_display(&state.root, path);
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) => {
            state.fail(display, format!("read {label}: {err}"));
            return None;
        }
    };
    let value = match serde_json::from_str::<Value>(&content) {
        Ok(value) => value,
        Err(err) => {
            state.fail(display, format!("parse {label} JSON: {err}"));
            return None;
        }
    };
    scan_private_json(&value, path, "$", state);
    match serde_json::from_value::<T>(value) {
        Ok(parsed) => Some(parsed),
        Err(err) => {
            state.fail(display, format!("validate {label} schema: {err}"));
            None
        }
    }
}

pub(super) fn collect_manifest_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    collect_manifest_paths_recursive(root, &mut paths)
        .with_context(|| format!("scan benchmark manifests under {}", root.display()))?;
    paths.sort();
    Ok(paths)
}

fn collect_manifest_paths_recursive(dir: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("read directory {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_manifest_paths_recursive(&path, paths)?;
        } else if path.extension().is_some_and(|ext| ext == "json")
            && path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == "manifests")
        {
            paths.push(path);
        }
    }
    Ok(())
}

fn resolve_public_path(state: &mut VerifyState, raw: &str, label: &str) -> Option<PathBuf> {
    let root = state.root.clone();
    scan_private_string(raw, &root.join(label), label, state);
    let path = Path::new(raw);
    if path.is_absolute() {
        state.fail(label.to_string(), "artifact path must be relative");
        return None;
    }
    if raw.trim().is_empty() {
        state.fail(label.to_string(), "artifact path must not be blank");
        return None;
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        state.fail(
            label.to_string(),
            "artifact path must stay inside benchmark root",
        );
        return None;
    }
    Some(root.join(path))
}

fn scan_private_json(value: &Value, path: &Path, pointer: &str, state: &mut VerifyState) {
    match value {
        Value::String(text) => scan_private_string(text, path, pointer, state),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                scan_private_json(item, path, &format!("{pointer}/{index}"), state);
            }
        }
        Value::Object(object) => {
            for (key, item) in object {
                scan_private_json(item, path, &format!("{pointer}/{key}"), state);
            }
        }
        Value::Bool(_) | Value::Number(_) | Value::Null => {}
    }
}

fn scan_private_string(text: &str, path: &Path, pointer: &str, state: &mut VerifyState) {
    if text.contains("~/.remem")
        || text.contains("$HOME/.remem")
        || text.contains("${HOME}/.remem")
        || contains_user_remem_path(text)
    {
        state.fail(
            rel_display(&state.root, path),
            format!("{pointer} contains a private remem path"),
        );
    }
    if let Some(home) = dirs::home_dir().and_then(|path| path.into_os_string().into_string().ok()) {
        if text.starts_with(&home) {
            state.fail(
                rel_display(&state.root, path),
                format!("{pointer} contains an absolute path under the current user home"),
            );
        }
    }
}

fn contains_user_remem_path(text: &str) -> bool {
    text.contains("/.remem/")
        && (text.contains("/Users/") || text.contains("/home/") || text.contains("/var/home/"))
}

fn rel_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

struct VerifyState {
    root: PathBuf,
    manifests_checked: usize,
    reports_checked: usize,
    run_artifacts_checked: usize,
    artifact_files: BTreeSet<String>,
    run_artifact_paths: BTreeSet<String>,
    coding_run_keys: BTreeSet<String>,
    coding_attempt_ids: BTreeSet<String>,
    failures: Vec<BenchVerifyFailure>,
}

impl VerifyState {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            manifests_checked: 0,
            reports_checked: 0,
            run_artifacts_checked: 0,
            artifact_files: BTreeSet::new(),
            run_artifact_paths: BTreeSet::new(),
            coding_run_keys: BTreeSet::new(),
            coding_attempt_ids: BTreeSet::new(),
            failures: Vec::new(),
        }
    }

    fn fail(&mut self, path: String, message: impl Into<String>) {
        self.failures.push(BenchVerifyFailure {
            path,
            message: message.into(),
        });
    }

    fn finish(self) -> BenchVerifyReport {
        BenchVerifyReport {
            schema_version: 1,
            root: self.root.to_string_lossy().into_owned(),
            passed: self.failures.is_empty(),
            manifests_checked: self.manifests_checked,
            reports_checked: self.reports_checked,
            run_artifacts_checked: self.run_artifacts_checked,
            artifact_files_checked: self.artifact_files.len(),
            failures: self.failures,
        }
    }
}
