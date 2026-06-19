use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};

use crate::db;

pub(super) enum ExistingKeyDatabaseState {
    Encrypted,
    Missing,
}

pub(super) fn inspect_existing_key_database(
    key_path: &Path,
    db_path: &Path,
) -> Result<ExistingKeyDatabaseState> {
    if !db_path.exists() {
        return Ok(ExistingKeyDatabaseState::Missing);
    }

    let key_text = std::fs::read_to_string(key_path)
        .with_context(|| format!("read SQLCipher key file {}", key_path.display()))?;
    let key = db::parse_cipher_key(&key_text)
        .with_context(|| format!("parse SQLCipher key file {}", key_path.display()))?
        .with_context(|| format!("SQLCipher key file is empty: {}", key_path.display()))?;

    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open existing remem database {}", db_path.display()))?;
    match db::configure_cipher(&conn, Some(&key)) {
        Ok(true) => return Ok(ExistingKeyDatabaseState::Encrypted),
        Ok(false) => bail!(
            "SQLCipher key was not applied while validating {}",
            db_path.display()
        ),
        Err(error) => {
            if sqlite_file_has_plaintext_header(db_path)? {
                bail!(
                    "SQLCipher key file exists at {} but {} is still plaintext SQLite; move the stale key aside and rerun `remem encrypt`, or restore the encrypted database that matches the key",
                    key_path.display(),
                    db_path.display()
                );
            }
            bail!(
                "SQLCipher key file exists at {} but does not unlock {}; key/DB mismatch or encrypted database corruption: {error:#}",
                key_path.display(),
                db_path.display()
            );
        }
    }
}

fn sqlite_file_has_plaintext_header(path: &Path) -> Result<bool> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("inspect {}", path.display()))?;
    let mut header = [0_u8; 16];
    match file.read_exact(&mut header) {
        Ok(()) => Ok(&header == b"SQLite format 3\0"),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::ScopedTestDataDir;

    #[test]
    fn run_encrypt_with_existing_key_verifies_encrypted_database() -> Result<()> {
        let test_dir = ScopedTestDataDir::new("encrypt-existing-key-encrypted-db");
        std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
        std::env::remove_var("REMEM_CIPHER_KEY");
        std::fs::create_dir_all(&test_dir.path)?;
        let raw_hex = "3".repeat(64);
        std::fs::write(test_dir.path.join(".key"), format!("v2:{raw_hex}"))?;
        {
            let conn = Connection::open(test_dir.db_path())?;
            db::configure_cipher(&conn, Some(&db::CipherKey::Raw(raw_hex.clone())))?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)", [])?;
        }

        super::super::maintenance::run_encrypt(false)?;

        let conn = Connection::open(test_dir.db_path())?;
        db::configure_cipher(&conn, Some(&db::CipherKey::Raw(raw_hex)))?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0))?;
        assert!(count > 0);
        Ok(())
    }

    #[test]
    fn run_encrypt_with_existing_key_rejects_plaintext_database() -> Result<()> {
        let test_dir = ScopedTestDataDir::new("encrypt-existing-key-plaintext-db");
        std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
        std::env::remove_var("REMEM_CIPHER_KEY");
        std::fs::create_dir_all(&test_dir.path)?;
        std::fs::write(test_dir.path.join(".key"), format!("v2:{}", "4".repeat(64)))?;
        {
            let conn = Connection::open(test_dir.db_path())?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)", [])?;
        }

        let error = super::super::maintenance::run_encrypt(false)
            .expect_err("plaintext DB with stale key must fail");

        let message = error.to_string();
        assert!(message.contains("plaintext SQLite"), "got: {message}");
        let header = std::fs::read(test_dir.db_path())?;
        assert_eq!(&header[..16], b"SQLite format 3\0");
        assert!(!test_dir.path.join("remem.db.bak").exists());
        Ok(())
    }

    #[test]
    fn run_encrypt_with_existing_key_rejects_mismatched_encrypted_database() -> Result<()> {
        let test_dir = ScopedTestDataDir::new("encrypt-existing-key-mismatch");
        std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
        std::env::remove_var("REMEM_CIPHER_KEY");
        std::fs::create_dir_all(&test_dir.path)?;
        let database_key = "5".repeat(64);
        let wrong_key = "6".repeat(64);
        std::fs::write(test_dir.path.join(".key"), format!("v2:{wrong_key}"))?;
        {
            let conn = Connection::open(test_dir.db_path())?;
            db::configure_cipher(&conn, Some(&db::CipherKey::Raw(database_key)))?;
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)", [])?;
        }

        let error = super::super::maintenance::run_encrypt(false)
            .expect_err("wrong key for encrypted DB must fail");

        let message = error.to_string();
        assert!(message.contains("does not unlock"), "got: {message}");
        assert!(!test_dir.path.join("remem.db.bak").exists());
        Ok(())
    }
}
