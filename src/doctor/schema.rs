use super::types::{Check, Status};
use crate::db;

pub(super) fn check_schema_migration() -> Check {
    let db_path = db::db_path();
    if !db_path.exists() {
        return Check {
            name: "Schema",
            status: Status::Ok,
            detail: "no database yet (will create on first use)".into(),
        };
    }

    let real_conn = match rusqlite::Connection::open(&db_path) {
        Ok(conn) => conn,
        Err(err) => {
            return Check {
                name: "Schema",
                status: Status::Fail,
                detail: format!("cannot open DB: {}", err),
            };
        }
    };
    if let Err(err) = real_conn.execute_batch("PRAGMA busy_timeout=5000;") {
        return Check {
            name: "Schema",
            status: Status::Fail,
            detail: format!("cannot set busy_timeout: {}", err),
        };
    }

    match crate::migrate::dry_run_pending(&real_conn) {
        Ok(result) => {
            if result.pending_count == 0 {
                Check {
                    name: "Schema",
                    status: Status::Ok,
                    detail: format!("v{} (up to date)", result.current_version),
                }
            } else if let Some(err) = result.error {
                Check {
                    name: "Schema",
                    status: Status::Fail,
                    detail: format!(
                        "{} pending migration(s) will FAIL: {}",
                        result.pending_count, err
                    ),
                }
            } else {
                Check {
                    name: "Schema",
                    status: Status::Ok,
                    detail: format!(
                        "v{} ({} pending migration(s), dry-run passed)",
                        result.current_version, result.pending_count
                    ),
                }
            }
        }
        Err(err) => Check {
            name: "Schema",
            status: Status::Fail,
            detail: format!("dry-run error: {}", err),
        },
    }
}
