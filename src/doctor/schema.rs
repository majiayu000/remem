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
                    format!(
                        "migrations v{} (sqlite user_version {}, up to date)",
                        result.migration_version, result.current_version
                    ),
                )
            } else {
                Check::new(
                    "Schema",
                    Status::Ok,
                    format!(
                        "migrations v{} (sqlite user_version {}, {} pending migration(s), dry-run passed)",
                        result.migration_version, result.current_version, result.pending_count
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

#[cfg(test)]
mod tests {
    use crate::db;
    use crate::db::test_support::ScopedTestDataDir;

    use super::*;

    #[test]
    fn schema_check_labels_migration_version_and_sqlite_user_version() -> anyhow::Result<()> {
        let test_dir = ScopedTestDataDir::new("doctor-schema-version-label");
        std::fs::create_dir_all(&test_dir.path)?;
        std::fs::write(test_dir.path.join(".key"), "doctor-schema-key")?;
        let conn = db::open_db()?;

        let check = check_schema_migration(Some(&conn), None);

        assert_eq!(check.icon(), "ok");
        assert!(
            check.detail.contains(&format!(
                "migrations v{}",
                crate::migrate::latest_schema_version()
            )),
            "got: {}",
            check.detail
        );
        assert!(
            check.detail.contains("sqlite user_version"),
            "got: {}",
            check.detail
        );
        assert!(check.detail.contains("up to date"), "got: {}", check.detail);
        Ok(())
    }
}
