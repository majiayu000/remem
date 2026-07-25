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
        assert_private_module_declaration(procedures_production, "write");
        assert_procedure_export_arm_calls_writer_once(procedures_production);

        let writer = read_repo_file("src/cli/actions/procedures/write.rs");
        assert_function_visibility(&writer, "run_procedure_export", "pub(super)");
        assert_function_visibility(&writer, "write_procedure_export_draft", "");

        let actions = read_repo_file("src/cli/actions.rs");
        assert!(
            !actions.contains("run_procedure_export"),
            "top-level CLI actions must not re-export the procedure export writer"
        );
        assert_cli_dispatch_routes_procedure_export_only_through_procedure_command();
        assert_no_cli_procedure_export_bridge("src/cli/actions/maintenance.rs");
        assert_no_procedure_export_cli_launches(&["plugins/remem/scripts/remem-hook.js"]);

        assert_no_background_export_writer_tokens(&[
            "src/worker.rs",
            "src/worker",
            "src/extraction_worker.rs",
            "src/session_rollup",
            "src/observation_extract.rs",
            "src/memory_candidate.rs",
            "src/memory_candidate",
            "src/user_context.rs",
            "src/user_context",
            "src/graph_candidate",
            "src/dream",
            "src/dream.rs",
            "src/hook_stdin.rs",
            "src/observe",
            "src/observe.rs",
            "src/context",
            "src/context.rs",
            "src/summarize",
            "src/summarize.rs",
            "src/mcp",
        ]);
    }

    fn assert_procedure_export_arm_calls_writer_once(content: &str) {
        let writer_call = "write::run_procedure_export(";
        let lines: Vec<&str> = content.lines().collect();
        let writer_call_lines: Vec<(usize, &str)> = lines
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, line)| line.contains(writer_call))
            .collect();
        assert_eq!(
            writer_call_lines.len(),
            1,
            "procedure export writer must be called exactly once"
        );
        let Some(export_arm_line) = lines
            .iter()
            .position(|line| line.trim() == "ProcedureAction::Export {")
        else {
            panic!("missing ProcedureAction::Export arm");
        };
        let (writer_line, writer_text) = writer_call_lines[0];
        assert!(
            writer_line > export_arm_line && writer_text.trim_start().starts_with("} => "),
            "procedure export writer call must be tied directly to the Export match arm"
        );
    }

    fn assert_private_module_declaration(content: &str, module: &str) {
        let expected = format!("mod {module};");
        let declarations: Vec<&str> = content
            .lines()
            .map(str::trim)
            .filter(|line| line.ends_with(&expected))
            .collect();
        assert_eq!(
            declarations,
            vec![expected.as_str()],
            "module `{module}` must have exactly one private declaration"
        );
    }

    fn assert_function_visibility(content: &str, name: &str, expected_visibility: &str) {
        let needle = format!("fn {name}(");
        let Some(line) = content.lines().find(|line| line.contains(&needle)) else {
            panic!("missing function declaration for {name}");
        };
        let expected_prefix = if expected_visibility.is_empty() {
            needle
        } else {
            format!("{expected_visibility} {needle}")
        };
        assert!(
            line.trim_start().starts_with(&expected_prefix),
            "function `{name}` must have visibility `{expected_visibility}`; found `{}`",
            line.trim()
        );
    }

    fn assert_cli_dispatch_routes_procedure_export_only_through_procedure_command() {
        let dispatch = read_repo_file("src/cli/dispatch.rs");
        assert!(
            !dispatch.contains("run_procedure_export")
                && !dispatch.contains("render_procedure_export")
                && !dispatch.contains("load_export_eligible_procedure"),
            "CLI dispatch must not call procedure export internals"
        );
        let run_procedures_lines: Vec<&str> = dispatch
            .lines()
            .map(str::trim)
            .filter(|line| line.contains("run_procedures"))
            .collect();
        assert_eq!(
            run_procedures_lines.len(),
            2,
            "`run_procedures` should appear only in the import list and Procedures command arm"
        );
        assert!(
            run_procedures_lines
                .contains(&"Commands::Procedures { action } => run_procedures(action)?,"),
            "CLI dispatch must route procedure actions only from Commands::Procedures"
        );
    }

    fn assert_no_cli_procedure_export_bridge(path: &str) {
        let content = read_repo_file(path);
        for token in ["run_procedures", "ProcedureAction::Export"] {
            assert!(
                !content.contains(token),
                "CLI wrapper {path} must not bridge background commands to procedure export via `{token}`"
            );
        }
    }

    fn assert_no_procedure_export_cli_launches(paths: &[&str]) {
        for path in paths {
            let content = read_repo_file(path);
            let lowercase = content.to_ascii_lowercase();
            let compact: String = lowercase.chars().filter(|ch| !ch.is_whitespace()).collect();
            for pattern in [
                "procedures export",
                "\"procedures\",\"export\"",
                "'procedures','export'",
                "`procedures`,`export`",
            ] {
                assert!(
                    !lowercase.contains(pattern) && !compact.contains(pattern),
                    "hook command surface {path} must not launch `procedures export`"
                );
            }
        }
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
