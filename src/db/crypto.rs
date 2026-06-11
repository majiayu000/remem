use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(crate) const ALLOW_PLAINTEXT_ENV: &str = "REMEM_ALLOW_PLAINTEXT_DB";
const RAW_KEY_PREFIX: &str = "v2:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CipherKey {
    Raw(String),
    Passphrase(String),
}

impl CipherKey {
    pub(crate) fn stored_value(&self) -> String {
        match self {
            CipherKey::Raw(hex) => format!("{RAW_KEY_PREFIX}{hex}"),
            CipherKey::Passphrase(value) => value.clone(),
        }
    }
}

pub(crate) fn parse_cipher_key(input: &str) -> Result<Option<CipherKey>> {
    let key = input.trim();
    if key.is_empty() {
        return Ok(None);
    }
    if let Some(hex) = key.strip_prefix(RAW_KEY_PREFIX) {
        validate_raw_key_hex(hex)?;
        return Ok(Some(CipherKey::Raw(hex.to_string())));
    }
    Ok(Some(CipherKey::Passphrase(key.to_string())))
}

pub(crate) fn load_cipher_key() -> Result<Option<CipherKey>> {
    if let Ok(key) = std::env::var("REMEM_CIPHER_KEY") {
        if !key.is_empty() {
            return parse_cipher_key(&key).context("parse REMEM_CIPHER_KEY");
        }
    }

    let key_path = super::core::data_dir().join(".key");
    if key_path.exists() {
        let key = std::fs::read_to_string(&key_path)
            .with_context(|| format!("read SQLCipher key file {}", key_path.display()))?;
        if let Some(parsed) = parse_cipher_key(&key)
            .with_context(|| format!("parse SQLCipher key file {}", key_path.display()))?
        {
            return Ok(Some(parsed));
        }
    }
    Ok(None)
}

pub(crate) fn plaintext_db_allowed() -> bool {
    std::env::var(ALLOW_PLAINTEXT_ENV).as_deref() == Ok("1")
}

pub(crate) fn require_cipher_key_or_plaintext_override() -> Result<Option<CipherKey>> {
    let key = load_cipher_key()?;
    if key.is_none() && !plaintext_db_allowed() {
        anyhow::bail!(
            "refusing to open remem database without a SQLCipher key; run `remem encrypt` to create a key and encrypted database, or set {ALLOW_PLAINTEXT_ENV}=1 to explicitly allow an unencrypted database"
        );
    }
    Ok(key)
}

pub(crate) fn configure_cipher(conn: &Connection, key: Option<&CipherKey>) -> Result<bool> {
    if let Some(key) = key {
        apply_cipher_key(conn, key)?;
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
    if let Some(key) = load_cipher_key()? {
        apply_cipher_key(conn, &key)?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn apply_cipher_key(conn: &Connection, key: &CipherKey) -> Result<()> {
    match key {
        CipherKey::Raw(hex) => apply_raw_key_pragma(conn, "key", hex),
        CipherKey::Passphrase(passphrase) => conn
            .pragma_update(None, "key", passphrase)
            .map_err(Into::into),
    }
}

pub(crate) fn rekey_connection_to_raw(conn: &Connection, hex: &str) -> Result<()> {
    apply_raw_key_pragma(conn, "rekey", hex)
}

fn apply_raw_key_pragma(conn: &Connection, pragma: &str, hex: &str) -> Result<()> {
    validate_raw_key_hex(hex)?;
    conn.execute_batch(&format!("PRAGMA {pragma} = \"x'{hex}'\";"))?;
    Ok(())
}

pub(crate) fn legacy_passphrase_to_raw_hex(passphrase: &str) -> Result<&str> {
    validate_raw_key_hex(passphrase)?;
    Ok(passphrase)
}

fn validate_raw_key_hex(hex: &str) -> Result<()> {
    if hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(());
    }
    anyhow::bail!("raw SQLCipher key must be exactly 64 hex characters");
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

    if let Err(e) = file.write_all(CipherKey::Raw(key.clone()).stored_value().as_bytes()) {
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

pub fn encrypt_database(key: &CipherKey) -> Result<()> {
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
    let attach_key = attach_key_sql(key)?;
    conn.execute(
        &format!(
            "ATTACH DATABASE '{}' AS encrypted KEY {}",
            encrypted_path_str.replace('\'', "''"),
            attach_key
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

fn attach_key_sql(key: &CipherKey) -> Result<String> {
    match key {
        CipherKey::Raw(hex) => {
            validate_raw_key_hex(hex)?;
            Ok(format!("\"x'{hex}'\""))
        }
        CipherKey::Passphrase(passphrase) => Ok(format!("'{}'", passphrase.replace('\'', "''"))),
    }
}

pub(crate) fn backup_cipher_key_file(key_path: &Path) -> Result<PathBuf> {
    let file_name = key_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid SQLCipher key file path {}", key_path.display()))?;
    let key_backup_path = key_path.with_file_name(format!("{file_name}.bak"));
    std::fs::copy(key_path, &key_backup_path).with_context(|| {
        format!(
            "backup SQLCipher key file {} to {}",
            key_path.display(),
            key_backup_path.display()
        )
    })?;
    Ok(key_backup_path)
}

pub(crate) fn write_raw_key_file(key_path: &Path, raw_hex: &str) -> Result<()> {
    validate_raw_key_hex(raw_hex)?;
    write_key_file_atomic(key_path, &format!("{RAW_KEY_PREFIX}{raw_hex}"))
}

fn write_key_file_atomic(path: &Path, contents: &str) -> Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid SQLCipher key file path {}", path.display()))?;
    let tmp_path = path.with_file_name(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    {
        use std::io::Write;
        #[cfg(unix)]
        let file = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .mode(0o600)
                .create_new(true)
                .write(true)
                .open(&tmp_path)
        };
        #[cfg(not(unix))]
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_path);
        let mut file =
            file.with_context(|| format!("create temp key file {}", tmp_path.display()))?;
        file.write_all(contents.as_bytes())
            .with_context(|| format!("write temp key file {}", tmp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("sync temp key file {}", tmp_path.display()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("set permissions on {}", tmp_path.display()))?;
    }
    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("replace SQLCipher key file {}", path.display()))?;
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
    fn parse_cipher_key_distinguishes_raw_and_legacy() -> Result<()> {
        let raw_hex = "a".repeat(64);
        assert_eq!(
            parse_cipher_key(&format!("v2:{raw_hex}"))?,
            Some(CipherKey::Raw(raw_hex.clone()))
        );
        assert_eq!(
            parse_cipher_key(&raw_hex)?,
            Some(CipherKey::Passphrase(raw_hex.clone()))
        );
        assert_eq!(parse_cipher_key("  \n")?, None);

        let err = parse_cipher_key("v2:not-hex")
            .expect_err("malformed raw key must fail closed")
            .to_string();
        assert!(err.contains("64 hex"), "got: {err}");
        Ok(())
    }

    #[test]
    fn generate_cipher_key_writes_64_hex_chars() -> Result<()> {
        let test_dir = ScopedTestDataDir::new("cipher-key");
        std::fs::create_dir_all(&test_dir.path)?;
        std::env::remove_var("REMEM_CIPHER_KEY");

        let key = generate_cipher_key()?;
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|ch| ch.is_ascii_hexdigit()));

        let saved = std::fs::read_to_string(test_dir.path.join(".key"))?;
        assert_eq!(saved, format!("v2:{key}"));
        Ok(())
    }

    #[test]
    fn raw_key_file_opens_existing_database() -> Result<()> {
        let test_dir = ScopedTestDataDir::new("cipher-raw-open");
        std::fs::create_dir_all(&test_dir.path)?;
        std::env::remove_var("REMEM_CIPHER_KEY");
        let raw_hex = "1".repeat(64);
        std::fs::write(test_dir.path.join(".key"), format!("v2:{raw_hex}"))?;

        {
            let conn = Connection::open(test_dir.db_path())?;
            configure_cipher(&conn, Some(&CipherKey::Raw(raw_hex.clone())))?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)", [])?;
            conn.execute("INSERT INTO t (v) VALUES ('raw-ok')", [])?;
        }

        let header = std::fs::read(test_dir.db_path())?;
        assert_ne!(&header[..16], b"SQLite format 3\0");

        let conn = crate::db::open_db_read_only()?;
        let value: String = conn.query_row("SELECT v FROM t WHERE id = 1", [], |row| row.get(0))?;
        assert_eq!(value, "raw-ok");
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
        encrypt_database(&CipherKey::Raw(key.clone()))?;

        assert!(
            test_dir.path.join("remem.db.bak").exists(),
            "backup should exist after encrypt"
        );
        assert!(db_path.exists(), "encrypted db should be at original path");
        let conn = Connection::open(db_path)?;
        configure_cipher(&conn, Some(&CipherKey::Raw(key)))?;
        let value: String = conn.query_row("SELECT v FROM t WHERE id = 1", [], |row| row.get(0))?;
        assert_eq!(value, "hello");
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
