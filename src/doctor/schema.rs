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

    let real_conn = match db::open_configured_connection(&db_path, key.as_ref()) {
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

pub(super) fn check_key_format() -> Check {
    if let Ok(env_key) = std::env::var("REMEM_CIPHER_KEY") {
        if !env_key.trim().is_empty() {
            return check_key_format_value("REMEM_CIPHER_KEY", &env_key, false);
        }
    }

    let key_path = db::data_dir().join(".key");
    if !key_path.exists() {
        return Check {
            name: "Key format",
            status: Status::Ok,
            detail: "no SQLCipher key file yet".into(),
        };
    }

    let key_text = match std::fs::read_to_string(&key_path) {
        Ok(key_text) => key_text,
        Err(error) => {
            return Check {
                name: "Key format",
                status: Status::Fail,
                detail: format!("cannot read {}: {}", key_path.display(), error),
            };
        }
    };
    let source = key_path.display().to_string();
    check_key_format_value(&source, &key_text, true)
}

fn check_key_format_value(source: &str, key_text: &str, is_file: bool) -> Check {
    match db::parse_cipher_key(key_text) {
        Ok(Some(db::CipherKey::Raw(_))) => Check {
            name: "Key format",
            status: Status::Ok,
            detail: format!("raw-key format (v2) from {source}"),
        },
        Ok(Some(db::CipherKey::Passphrase(_))) => Check {
            name: "Key format",
            status: Status::Warn,
            detail: if is_file {
                format!("legacy passphrase key format in {source}; run `remem encrypt --rekey-raw`")
            } else {
                format!(
                    "legacy passphrase key format in {source}; use v2:<64hex> or unset {source} and run `remem encrypt --rekey-raw`"
                )
            },
        },
        Ok(None) => Check {
            name: "Key format",
            status: Status::Fail,
            detail: if is_file {
                format!("empty SQLCipher key file at {source}")
            } else {
                format!("empty SQLCipher key in {source}")
            },
        },
        Err(error) => Check {
            name: "Key format",
            status: Status::Fail,
            detail: format!("invalid SQLCipher key in {source}: {error}"),
        },
    }
}
