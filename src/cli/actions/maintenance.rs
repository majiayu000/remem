use anyhow::Result;

use crate::{db, memory};

pub(in crate::cli) fn run_encrypt() -> Result<()> {
    let key_path = db::data_dir().join(".key");
    if key_path.exists() {
        println!(
            "Database is already encrypted (key file exists at {})",
            key_path.display()
        );
        return Ok(());
    }

    println!("Generating encryption key...");
    let key = db::generate_cipher_key()?;
    println!("Key saved to {}", key_path.display());

    println!("Encrypting database (this may take a moment)...");
    db::encrypt_database(&key)?;

    println!("Done. Database is now encrypted with SQLCipher.");
    println!("Backup saved as remem.db.bak");
    Ok(())
}

pub(in crate::cli) fn run_cleanup() -> Result<()> {
    let conn = db::open_db()?;
    let events_deleted = memory::cleanup_old_events(&conn, 30)?;
    let memories_archived = memory::archive_stale_memories(&conn, 180)?;
    println!("Cleanup complete:");
    println!("  Old events deleted (>30 days): {}", events_deleted);
    println!(
        "  Stale memories archived (>180 days): {}",
        memories_archived
    );
    Ok(())
}
