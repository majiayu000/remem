use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::time::Duration;

use anyhow::Result;
use rusqlite::{Connection, Transaction, TransactionBehavior};

use super::*;

#[test]
fn concurrent_same_activation_serializes_and_replays_the_winner() -> Result<()> {
    struct DbFiles(std::path::PathBuf);
    impl Drop for DbFiles {
        fn drop(&mut self) {
            crate::db::test_support::cleanup_temp_db_files(&self.0);
        }
    }
    let path = crate::db::test_support::unique_temp_db_path("activation-race");
    let _db_files = DbFiles(path.clone());
    let seed = Connection::open(&path)?;
    seed.pragma_update(None, "journal_mode", "WAL")?;
    crate::migrate::run_migrations(&seed)?;
    drop(seed);

    let request = Arc::new(super::tests::request("save:concurrent", "same"));
    let barrier = Arc::new(Barrier::new(2));
    let writes = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let path = path.clone();
        let request = Arc::clone(&request);
        let barrier = Arc::clone(&barrier);
        let writes = Arc::clone(&writes);
        handles.push(std::thread::spawn(
            move || -> Result<ActiveMemoryWriteResult> {
                let conn = Connection::open(path)?;
                conn.busy_timeout(Duration::from_secs(30))?;
                barrier.wait();
                execute_one(&conn, &request, |_| {
                    writes.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(150));
                    super::tests::insert_memory(&conn, "same")
                })
            },
        ));
    }
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("activation thread panicked"))
        .collect::<Result<Vec<_>>>()?;

    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert_eq!(results[0].memory_id, results[1].memory_id);
    assert_eq!(results.iter().filter(|result| result.replayed).count(), 1);
    Ok(())
}

#[test]
fn caller_owned_immediate_transactions_serialize_before_receipt_lookup() -> Result<()> {
    struct DbFiles(std::path::PathBuf);
    impl Drop for DbFiles {
        fn drop(&mut self) {
            crate::db::test_support::cleanup_temp_db_files(&self.0);
        }
    }
    let path = crate::db::test_support::unique_temp_db_path("activation-caller-race");
    let _db_files = DbFiles(path.clone());
    let seed = Connection::open(&path)?;
    seed.pragma_update(None, "journal_mode", "WAL")?;
    crate::migrate::run_migrations(&seed)?;
    drop(seed);

    let request = Arc::new(super::tests::request("save:caller-concurrent", "same"));
    let barrier = Arc::new(Barrier::new(2));
    let writes = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let path = path.clone();
        let request = Arc::clone(&request);
        let barrier = Arc::clone(&barrier);
        let writes = Arc::clone(&writes);
        handles.push(std::thread::spawn(
            move || -> Result<ActiveMemoryWriteResult> {
                let conn = Connection::open(path)?;
                conn.busy_timeout(Duration::from_secs(30))?;
                barrier.wait();
                let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate)?;
                let _: i64 = tx.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
                let result = execute_one(&tx, &request, |_| {
                    writes.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(150));
                    super::tests::insert_memory(&tx, "same")
                })?;
                tx.commit()?;
                Ok(result)
            },
        ));
    }
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("activation thread panicked"))
        .collect::<Result<Vec<_>>>()?;

    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert_eq!(results[0].memory_id, results[1].memory_id);
    assert_eq!(results.iter().filter(|result| result.replayed).count(), 1);
    Ok(())
}
