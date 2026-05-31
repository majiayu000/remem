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

    let key = match db::require_cipher_key_or_plaintext_override() {
        Ok(key) => key,
        Err(err) => {
            return Check {
                name: "Schema",
                status: Status::Fail,
                detail: format!("cannot open DB: {}", err),
            };
        }
    };

    let real_conn = match db::open_configured_connection(&db_path, key.as_deref()) {
        Ok(conn) => conn,
        Err(err) => {
            return Check {
                name: "Schema",
                status: Status::Fail,
                detail: format!("cannot open DB: {}", err),
            };
        }
    };
    match crate::migrate::dry_run_pending(&real_conn) {
        Ok(result) => {
            if let Some(err) = result.error {
                Check {
                    name: "Schema",
                    status: Status::Fail,
                    detail: if result.pending_count == 0 {
                        format!("schema check failed: {}", err)
                    } else {
                        format!(
                            "{} pending migration(s) will FAIL: {}",
                            result.pending_count, err
                        )
                    },
                }
            } else if result.pending_count == 0 {
                Check {
                    name: "Schema",
                    status: Status::Ok,
                    detail: format!("v{} (up to date)", result.current_version),
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
