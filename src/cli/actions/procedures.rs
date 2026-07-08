use anyhow::Result;
use serde::Serialize;

use crate::{db, memory::procedure::ProcedureListItem};

use super::super::procedure_types::ProcedureAction;

mod write;

pub(in crate::cli) fn run_procedures(action: ProcedureAction) -> Result<()> {
    match action {
        ProcedureAction::List {
            project,
            limit,
            offset,
            json,
        } => run_procedure_list(project.as_deref(), limit, offset, json),
        ProcedureAction::Export {
            memory_id,
            format,
            out,
            overwrite_generated,
        } => write::run_procedure_export(
            memory_id,
            format.into(),
            out.as_deref(),
            overwrite_generated,
        ),
    }
}

fn run_procedure_list(project: Option<&str>, limit: i64, offset: i64, json: bool) -> Result<()> {
    let conn = db::open_db()?;
    let procedures =
        crate::memory::procedure::list_promoted_procedures(&conn, project, limit, offset)?;
    if json {
        let output = ProcedureListJson {
            project: project.map(str::to_string),
            limit: normalized_limit(limit),
            offset: offset.max(0),
            count: procedures.len(),
            procedures,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }
    print!("{}", render_procedure_list(&procedures));
    Ok(())
}

fn render_procedure_list(procedures: &[ProcedureListItem]) -> String {
    if procedures.is_empty() {
        return "No promoted procedures found.\n".to_string();
    }
    let mut output = format!("Found {} promoted procedure(s):\n\n", procedures.len());
    for procedure in procedures {
        output.push_str(&format!(
            "#{} {} [{} run(s), last verified: {}]\n",
            procedure.id,
            procedure.title,
            procedure.verified_runs,
            procedure
                .last_verification_epoch
                .map(format_epoch)
                .unwrap_or_else(|| "unknown".to_string())
        ));
        output.push_str(&format!("  Project: {}\n", procedure.project));
        if let Some(branch) = &procedure.branch {
            output.push_str(&format!("  Branch:  {branch}\n"));
        }
        if let Some(command) = &procedure.command {
            output.push_str(&format!("  Command: {command}\n"));
        }
        output.push_str(&format!(
            "  Confidence: {}\n",
            procedure
                .confidence
                .map(|confidence| format!("{confidence:.2}"))
                .unwrap_or_else(|| "unknown".to_string())
        ));
        output.push_str(&format!(
            "  Files:   {} touched\n",
            procedure.files_touched_count
        ));
    }
    output
}

fn normalized_limit(limit: i64) -> i64 {
    if limit <= 0 {
        50
    } else {
        limit.min(500)
    }
}

fn format_epoch(epoch: i64) -> String {
    chrono::DateTime::from_timestamp(epoch, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default()
}

#[derive(Debug, Serialize)]
struct ProcedureListJson {
    project: Option<String>,
    limit: i64,
    offset: i64,
    count: usize,
    procedures: Vec<ProcedureListItem>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn render_procedure_list_shows_maturity_columns() {
        let rendered = render_procedure_list(&[ProcedureListItem {
            id: 7,
            title: "Procedure: release-check".to_string(),
            project: "/tmp/remem".to_string(),
            branch: Some("main".to_string()),
            topic_key: Some("procedure-release-check".to_string()),
            command: Some("cargo test".to_string()),
            reuse_condition: Some("same project".to_string()),
            files_touched: vec!["src/lib.rs".to_string()],
            files_touched_count: 1,
            verified_runs: 2,
            last_verification_epoch: Some(1_200),
            confidence: Some(0.86),
        }]);

        assert!(rendered.contains("#7 Procedure: release-check [2 run(s)"));
        assert!(rendered.contains("Project: /tmp/remem"));
        assert!(rendered.contains("Branch:  main"));
        assert!(rendered.contains("Command: cargo test"));
        assert!(rendered.contains("Confidence: 0.86"));
        assert!(rendered.contains("Files:   1 touched"));
    }

    #[test]
    fn export_writer_is_reachable_only_from_cli_procedure_action() {
        let procedures_action = read_repo_file("src/cli/actions/procedures.rs");
        let procedures_production = production_section(&procedures_action);
        assert!(
            procedures_production.contains("mod write;"),
            "procedure export writer must stay private to the procedures action module"
        );
        assert!(
            !procedures_production.contains("pub mod write"),
            "procedure export writer module must not be public"
        );
        assert!(
            procedures_production.contains("write::run_procedure_export("),
            "CLI procedures export action should be the only runtime entrypoint"
        );

        let writer = read_repo_file("src/cli/actions/procedures/write.rs");
        assert!(
            writer.contains("pub(super) fn run_procedure_export"),
            "procedure export entrypoint must stay visible only to its parent CLI action"
        );
        assert!(
            writer.contains("fn write_procedure_export_draft"),
            "draft writer must stay module-private"
        );
        assert!(
            !writer.contains("pub(crate) fn write_procedure_export_draft")
                && !writer.contains("pub fn write_procedure_export_draft"),
            "draft writer must not become reachable from non-CLI modules"
        );

        let actions = read_repo_file("src/cli/actions.rs");
        assert!(
            !actions.contains("run_procedure_export"),
            "top-level CLI actions must not re-export the procedure export writer"
        );

        assert_no_background_export_writer_tokens(&[
            "src/worker.rs",
            "src/worker",
            "src/extraction_worker.rs",
            "src/session_rollup",
            "src/observation_extract.rs",
            "src/memory_candidate",
            "src/user_context",
            "src/graph_candidate",
            "src/dream",
            "src/observe",
            "src/context",
            "src/summarize",
            "src/mcp",
        ]);
    }

    fn assert_no_background_export_writer_tokens(paths: &[&str]) {
        let forbidden = [
            "run_procedure_export",
            "write_procedure_export_draft",
            "ProcedureExportWriteRequest",
            "ProcedureExportWriteResult",
            "render_procedure_export",
            "load_export_eligible_procedure",
        ];
        for path in paths {
            let path = repo_root().join(path);
            assert_path_has_no_tokens(&path, &forbidden);
        }
    }

    fn assert_path_has_no_tokens(path: &Path, forbidden: &[&str]) {
        if path.is_file() {
            assert_file_has_no_tokens(path, forbidden);
            return;
        }
        let entries = std::fs::read_dir(path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for entry in entries {
            let entry =
                entry.unwrap_or_else(|error| panic!("read {} entry: {error}", path.display()));
            let path = entry.path();
            if path.is_dir() {
                assert_path_has_no_tokens(&path, forbidden);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                assert_file_has_no_tokens(&path, forbidden);
            }
        }
    }

    fn assert_file_has_no_tokens(path: &Path, forbidden: &[&str]) {
        let content = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for token in forbidden {
            assert!(
                !content.contains(token),
                "background path {} must not reference procedure export writer token `{}`",
                path.display(),
                token
            );
        }
    }

    fn read_repo_file(path: &str) -> String {
        let path = repo_root().join(path);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    }

    fn production_section(content: &str) -> &str {
        content.split("#[cfg(test)]").next().unwrap_or(content)
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }
}
