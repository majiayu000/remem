use rusqlite::Connection;

use super::types::{Check, Status};

pub(super) fn check_procedure_exports(conn: Option<&Connection>) -> Check {
    let Some(conn) = conn else {
        return Check::new("Procedure exports", Status::Warn, "cannot open database");
    };

    match crate::memory::procedure::procedure_export_registry_exists(conn) {
        Ok(true) => {}
        Ok(false) => {
            return Check::new(
                "Procedure exports",
                Status::Ok,
                "procedure export registry not migrated yet",
            );
        }
        Err(error) => {
            return Check::new(
                "Procedure exports",
                Status::Warn,
                format!("cannot inspect procedure export registry: {error}"),
            );
        }
    }

    let report = match crate::memory::procedure::load_procedure_export_doctor_report(
        conn,
        chrono::Utc::now().timestamp(),
    ) {
        Ok(report) => report,
        Err(error) => {
            return Check::new(
                "Procedure exports",
                Status::Warn,
                format!("cannot load procedure export registry: {error}"),
            );
        }
    };

    if report.total_exports == 0 {
        return Check::new(
            "Procedure exports",
            Status::Ok,
            "no review-gated procedure exports recorded",
        );
    }

    let mut detail = format!(
        "{} export(s) across {} project(s)",
        report.total_exports, report.project_count
    );
    if !report.project_exports.is_empty() {
        let project_counts = report
            .project_exports
            .iter()
            .map(|project| format!("{}={}", project.project, project.exports))
            .collect::<Vec<_>>()
            .join(", ");
        detail.push_str(&format!("; projects: {project_counts}"));
    }
    if report.drifted_exports() == 0 {
        return Check::new("Procedure exports", Status::Ok, detail);
    }

    detail.push_str(&format!(
        "; drifted={} inactive={} stale={} changed={}",
        report.drifted_exports(),
        report.inactive,
        report.stale,
        report.changed
    ));
    if !report.examples.is_empty() {
        let examples = report
            .examples
            .iter()
            .map(|example| {
                format!(
                    "#{} {} {} ({})",
                    example.memory_id,
                    example.format,
                    example.output_path,
                    example.reason.as_str()
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        detail.push_str(&format!("; examples: {examples}"));
    }

    Check::new("Procedure exports", Status::Warn, detail)
}
