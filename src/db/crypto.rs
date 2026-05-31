use anyhow::Result;
use rusqlite::Connection;

pub(crate) const ALLOW_PLAINTEXT_ENV: &str = "REMEM_ALLOW_PLAINTEXT_DB";

pub(crate) fn load_cipher_key() -> Option<String> {
    if let Ok(key) = std::env::var("REMEM_CIPHER_KEY") {
        if !key.is_empty() {
            return Some(key);
        }
    }

    let key_path = super::core::data_dir().join(".key");
    if key_path.exists() {
        if let Ok(key) = std::fs::read_to_string(&key_path) {
            let key = key.trim().to_string();
            if !key.is_empty() {
                return Some(key);
            }
        }
    }
    None
}

pub(crate) fn plaintext_db_allowed() -> bool {
    std::env::var(ALLOW_PLAINTEXT_ENV).as_deref() == Ok("1")
}

pub(crate) fn require_cipher_key_or_plaintext_override() -> Result<Option<String>> {
    let key = load_cipher_key();
    if key.is_none() && !plaintext_db_allowed() {
        anyhow::bail!(
            "refusing to open remem database without a SQLCipher key; run `remem encrypt` to create a key and encrypted database, or set {ALLOW_PLAINTEXT_ENV}=1 to explicitly allow an unencrypted database"
        );
    }
    Ok(key)
}

pub(crate) fn configure_cipher(conn: &Connection, key: Option<&str>) -> Result<bool> {
    if let Some(key) = key {
        conn.pragma_update(None, "key", key)?;
        if !can_read_schema(conn) {
            anyhow::bail!("SQLCipher key was applied but the database schema is unreadable");
        }
        return Ok(true);
    }

    crate::log::error(
        "db",
        &format!("opening unencrypted remem database because {ALLOW_PLAINTEXT_ENV}=1 is set"),
    );
    Ok(false)
}

pub(crate) fn apply_cipher_key_if_available(conn: &Connection) -> Result<bool> {
    if let Some(key) = load_cipher_key() {
        conn.pragma_update(None, "key", &key)?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn can_read_schema(conn: &Connection) -> bool {
    conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    })
    .is_ok()
}

pub fn generate_cipher_key() -> Result<String> {
    generate_cipher_key_with(getrandom::fill)
}

fn generate_cipher_key_with<F>(fill_random: F) -> Result<String>
where
    F: FnOnce(&mut [u8]) -> std::result::Result<(), getrandom::Error>,
{
    use std::io::Write;

    let mut key_bytes = [0u8; 32];
    fill_random(&mut key_bytes).map_err(|e| {
        anyhow::anyhow!(
            "OS randomness unavailable while generating cipher key: {}",
            e
        )
    })?;
    let key: String = key_bytes
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect();

    let data_dir = super::core::data_dir();
    std::fs::create_dir_all(&data_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir_perms = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(&data_dir, dir_perms).map_err(|e| {
            anyhow::anyhow!(
                "cannot set data dir permissions to 0700 ({}): {}",
                data_dir.display(),
                e
            )
        })?;
    }

    let key_path = data_dir.join(".key");

    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .mode(0o600)
            .create_new(true)
            .write(true)
            .open(&key_path)
            .map_err(|e| {
                anyhow::anyhow!(
                    "cannot create cipher key file at {}: {}",
                    key_path.display(),
                    e
                )
            })?
    };

    #[cfg(not(unix))]
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&key_path)
        .map_err(|e| {
            anyhow::anyhow!(
                "cannot create cipher key file at {}: {}",
                key_path.display(),
                e
            )
        })?;

    if let Err(e) = file.write_all(key.as_bytes()) {
        drop(file);
        let _ = std::fs::remove_file(&key_path);
        return Err(anyhow::anyhow!(
            "failed to write cipher key to {}: {}",
            key_path.display(),
            e
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file_perms = std::fs::Permissions::from_mode(0o600);
        if let Err(e) = std::fs::set_permissions(&key_path, file_perms) {
            drop(file);
            let _ = std::fs::remove_file(&key_path);
            return Err(anyhow::anyhow!(
                "cannot enforce 0600 on cipher key file {}: {} (key file removed)",
                key_path.display(),
                e
            ));
        }
    }

    Ok(key)
}

pub fn encrypt_database(key: &str) -> Result<()> {
    let db_file = super::core::db_path();
    if !db_file.exists() {
        anyhow::bail!("database not found: {}", db_file.display());
    }

    let encrypted_path = db_file.with_extension("db.enc");
    let encrypted_path_str = encrypted_path.to_str().ok_or_else(|| {
        anyhow::anyhow!(
            "encrypted database path is not valid UTF-8: {}",
            encrypted_path.display()
        )
    })?;
    if encrypted_path_str.contains('\0') {
        anyhow::bail!(
            "encrypted database path contains a NUL byte: {}",
            encrypted_path.display()
        );
    }
    let conn = Connection::open(&db_file)?;
    // Enforce foreign keys so ON DELETE CASCADE / SET NULL behave during the
    // sqlcipher_export copy; foreign_keys defaults to OFF on every new
    // connection (#244).
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
    conn.execute(
        &format!(
            "ATTACH DATABASE '{}' AS encrypted KEY '{}'",
            encrypted_path_str.replace('\'', "''"),
            key.replace('\'', "''")
        ),
        [],
    )?;
    conn.query_row("SELECT sqlcipher_export('encrypted')", [], |_| Ok(()))?;
    conn.execute("DETACH DATABASE encrypted", [])?;
    drop(conn);

    let backup_path = db_file.with_extension("db.bak");
    std::fs::rename(&db_file, &backup_path)?;
    std::fs::rename(&encrypted_path, &db_file)?;

    crate::log::info(
        "encrypt",
        &format!("database encrypted, backup at {}", backup_path.display()),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::ScopedTestDataDir;

    #[test]
    fn open_db_refuses_plaintext_without_explicit_override() {
        let test_dir = ScopedTestDataDir::new("cipher-fail-closed");
        std::env::remove_var(ALLOW_PLAINTEXT_ENV);

        let err = match crate::db::open_db() {
            Ok(_) => panic!("open_db must fail closed without a cipher key"),
            Err(err) => err,
        };

        let message = err.to_string();
        assert!(message.contains("SQLCipher key"), "got: {message}");
        assert!(
            message.contains(ALLOW_PLAINTEXT_ENV),
            "override must be explicit: {message}"
        );
        assert!(
            !test_dir.db_path().exists(),
            "fail-closed path must not create a plaintext database"
        );
    }

    #[test]
    fn open_db_allows_plaintext_only_with_explicit_override() -> Result<()> {
        let test_dir = ScopedTestDataDir::new("cipher-plaintext-override");

        let conn = crate::db::open_db()?;
        let table_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0))?;
        assert!(table_count > 0);
        drop(conn);

        let header = std::fs::read(test_dir.db_path())?;
        assert_eq!(&header[..16], b"SQLite format 3\0");
        let log = std::fs::read_to_string(test_dir.path.join("remem.log"))?;
        assert!(log.contains("opening unencrypted remem database"));
        Ok(())
    }

    #[test]
    fn generate_cipher_key_writes_64_hex_chars() -> Result<()> {
        let test_dir = ScopedTestDataDir::new("cipher-key");
        std::fs::create_dir_all(&test_dir.path)?;

        let key = generate_cipher_key()?;
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|ch| ch.is_ascii_hexdigit()));

        let saved = std::fs::read_to_string(test_dir.path.join(".key"))?;
        assert_eq!(saved, key);
        Ok(())
    }

    #[test]
    fn generate_cipher_key_fails_when_os_randomness_is_unavailable() {
        let test_dir = ScopedTestDataDir::new("cipher-key-fail");
        std::fs::create_dir_all(&test_dir.path).expect("test data dir should exist");

        let err = generate_cipher_key_with(|_| Err(getrandom::Error::UNSUPPORTED))
            .expect_err("cipher key generation should fail without OS randomness");

        assert!(err.to_string().contains("OS randomness unavailable"));
        assert!(!test_dir.path.join(".key").exists());
    }

    #[cfg(unix)]
    #[test]
    fn generate_cipher_key_writes_file_with_0600_and_dir_with_0700() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let test_dir = ScopedTestDataDir::new("cipher-key-perms");
        std::fs::create_dir_all(&test_dir.path)?;

        let _ = generate_cipher_key()?;

        let file_mode = std::fs::metadata(test_dir.path.join(".key"))?
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600, "key file must be 0600");

        let dir_mode = std::fs::metadata(&test_dir.path)?.permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "data dir must be 0700");
        Ok(())
    }

    #[test]
    fn encrypt_database_escapes_single_quote_in_path() -> Result<()> {
        let test_dir = ScopedTestDataDir::new("encrypt-quote'path");
        std::fs::create_dir_all(&test_dir.path)?;

        let db_path = test_dir.db_path();
        {
            let conn = Connection::open(&db_path)?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)", [])?;
            conn.execute("INSERT INTO t (v) VALUES ('hello')", [])?;
        }

        let key = generate_cipher_key()?;
        encrypt_database(&key)?;

        assert!(
            test_dir.path.join("remem.db.bak").exists(),
            "backup should exist after encrypt"
        );
        assert!(db_path.exists(), "encrypted db should be at original path");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn generate_cipher_key_refuses_to_overwrite_existing_key() -> Result<()> {
        use std::io::Write;

        let test_dir = ScopedTestDataDir::new("cipher-key-no-overwrite");
        std::fs::create_dir_all(&test_dir.path)?;
        let key_path = test_dir.path.join(".key");
        let mut existing = std::fs::File::create(&key_path)?;
        existing.write_all(b"preexisting-key")?;
        drop(existing);

        let err =
            generate_cipher_key().expect_err("must not overwrite an existing cipher key file");
        assert!(
            err.to_string().contains("cannot create cipher key file"),
            "unexpected error: {}",
            err
        );

        let preserved = std::fs::read_to_string(&key_path)?;
        assert_eq!(preserved, "preexisting-key");
        Ok(())
    }
}
