use anyhow::Result;
use rusqlite::Connection;

pub(super) fn load_cipher_key() -> Option<String> {
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

    std::fs::create_dir_all(super::core::data_dir())?;
    let key_path = super::core::data_dir().join(".key");
    let mut file = std::fs::File::create(&key_path)?;
    file.write_all(key.as_bytes())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        if let Err(e) = std::fs::set_permissions(&key_path, perms) {
            crate::log::warn("db", &format!("cannot set key file permissions: {}", e));
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
    let conn = Connection::open(&db_file)?;
    conn.execute_batch("PRAGMA busy_timeout=5000;")?;
    conn.execute(
        &format!(
            "ATTACH DATABASE '{}' AS encrypted KEY '{}'",
            encrypted_path.display(),
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
}
