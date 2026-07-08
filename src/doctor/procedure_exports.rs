use rusqlite::Connection;

use super::types::{Check, Status};

pub(super) fn check_procedure_exports(conn: Option<&Connection>) -> Check {
    let Some(conn) = conn else {
        return Check::new("Procedure exports", Status::Warn, "cannot open database");
    };

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
