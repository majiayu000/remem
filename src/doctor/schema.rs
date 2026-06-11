use super::types::{Check, Status};
use crate::db;
use rusqlite::Connection;

pub(super) fn check_schema_migration(conn: Option<&Connection>, open_error: Option<&str>) -> Check {
    let db_path = db::db_path();
    if !db_path.exists() {
        return Check::new(
            "Schema",
            Status::Ok,
            "no database yet (will create on first use)",
        );
    }

    let Some(conn) = conn else {
        return Check::new(
            "Schema",
            Status::Fail,
            format!(
                "cannot open DB: {}",
                open_error.unwrap_or("database connection unavailable")
            ),
        );
    };
    match crate::migrate::dry_run_pending(conn) {
        Ok(result) => {
            if let Some(err) = result.error {
                Check::new(
                    "Schema",
                    Status::Fail,
                    if result.pending_count == 0 {
                        format!("schema check failed: {}", err)
                    } else {
                        format!(
                            "{} pending migration(s) will FAIL: {}",
                            result.pending_count, err
                        )
                    },
                )
            } else if result.pending_count == 0 {
                Check::new(
                    "Schema",
                    Status::Ok,
                    format!("v{} (up to date)", result.current_version),
                )
            } else {
                Check::new(
                    "Schema",
                    Status::Ok,
                    format!(
                        "v{} ({} pending migration(s), dry-run passed)",
                        result.current_version, result.pending_count
                    ),
                )
            }
        }
        Err(err) => Check::new("Schema", Status::Fail, format!("dry-run error: {}", err)),
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
        return Check::new("Key format", Status::Ok, "no SQLCipher key file yet");
    }

    let key_text = match std::fs::read_to_string(&key_path) {
        Ok(key_text) => key_text,
        Err(error) => {
            return Check::new(
                "Key format",
                Status::Fail,
                format!("cannot read {}: {}", key_path.display(), error),
            );
        }
    };
    let source = key_path.display().to_string();
    check_key_format_value(&source, &key_text, true)
}

fn check_key_format_value(source: &str, key_text: &str, is_file: bool) -> Check {
    match db::parse_cipher_key(key_text) {
        Ok(Some(db::CipherKey::Raw(_))) => Check::new(
            "Key format",
            Status::Ok,
            format!("raw-key format (v2) from {source}"),
        ),
        Ok(Some(db::CipherKey::Passphrase(_))) => Check::new(
            "Key format",
            Status::Warn,
            if is_file {
                format!("legacy passphrase key format in {source}; run `remem encrypt --rekey-raw`")
            } else {
                format!(
                    "legacy passphrase key format in {source}; use v2:<64hex> or unset {source} and run `remem encrypt --rekey-raw`"
                )
            },
        ),
        Ok(None) => Check::new(
            "Key format",
            Status::Fail,
            if is_file {
                format!("empty SQLCipher key file at {source}")
            } else {
                format!("empty SQLCipher key in {source}")
            },
        ),
        Err(error) => Check::new(
            "Key format",
            Status::Fail,
            format!("invalid SQLCipher key in {source}: {error}"),
        ),
    }
}
