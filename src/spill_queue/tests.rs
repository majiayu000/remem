use std::sync::{Arc, Barrier};

use super::*;

fn queue(label: &str) -> Result<(crate::db::test_support::ScopedTestDataDir, SpillQueue)> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new(label);
    let queue = SpillQueue::new(data_dir.path.join("capture-spill.jsonl"))?;
    Ok((data_dir, queue))
}

#[test]
fn append_after_claim_survives_claim_completion() -> Result<()> {
    let (_data_dir, queue) = queue("spill-queue-append-after-claim")?;
    queue.append_line(b"claimed-a")?;
    let claim = queue
        .claim(Duration::from_secs(60))?
        .context("active queue should be claimable")?;
    queue.append_line(b"new-b")?;

    claim.finish()?;

    assert_eq!(std::fs::read_to_string(&queue.active_path)?, "new-b\n");
    assert!(!claim.path().exists());
    Ok(())
}

#[test]
fn concurrent_claimers_only_one_owns_active_spill() -> Result<()> {
    let (_data_dir, queue) = queue("spill-queue-concurrent-claim")?;
    queue.append_line(b"only-once")?;
    let barrier = Arc::new(Barrier::new(3));
    let handles = (0..2)
        .map(|_| {
            let queue = queue.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || -> Result<Option<SpillClaim>> {
                barrier.wait();
                queue.claim(Duration::from_secs(60))
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let mut claims = handles
        .into_iter()
        .map(|handle| handle.join().expect("claim thread should not panic"))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    assert_eq!(claims.len(), 1);
    claims.pop().unwrap().restore()?;
    assert_eq!(std::fs::read_to_string(&queue.active_path)?, "only-once\n");
    Ok(())
}

#[test]
fn failed_claim_records_merge_without_overwriting_new_appends() -> Result<()> {
    let (_data_dir, queue) = queue("spill-queue-failed-merge")?;
    queue.append_line(b"claimed-a")?;
    let claim = queue
        .claim(Duration::from_secs(60))?
        .context("active queue should be claimable")?;
    queue.append_line(b"new-b")?;
    SpillQueue::new(claim.failed_path().to_path_buf())?.append_line(b"failed-a")?;

    claim.finish()?;

    assert_eq!(
        std::fs::read_to_string(&queue.active_path)?,
        "new-b\nfailed-a\n"
    );
    Ok(())
}

#[test]
fn stale_orphan_claim_is_restored() -> Result<()> {
    let (_data_dir, queue) = queue("spill-queue-orphan")?;
    let orphan = queue
        .active_path
        .with_file_name("capture-spill.replay-99999999-0-0.jsonl");
    std::fs::create_dir_all(orphan.parent().context("orphan path should have parent")?)?;
    std::fs::write(&orphan, "orphan-a\n")?;

    assert_eq!(queue.restore_orphaned_claims(Duration::ZERO)?, 1);
    assert_eq!(std::fs::read_to_string(&queue.active_path)?, "orphan-a\n");
    assert!(!orphan.exists());
    Ok(())
}

#[test]
fn old_dead_claim_is_restored_after_minimum_age() -> Result<()> {
    let (_data_dir, queue) = queue("spill-queue-old-dead-orphan")?;
    let orphan = queue
        .active_path
        .with_file_name("capture-spill.replay-99999999-0-0.jsonl");
    std::fs::create_dir_all(orphan.parent().context("orphan path should have parent")?)?;
    std::fs::write(&orphan, "orphan-a\n")?;

    assert_eq!(queue.restore_orphaned_claims(Duration::from_secs(60))?, 1);
    assert_eq!(std::fs::read_to_string(&queue.active_path)?, "orphan-a\n");
    assert!(!orphan.exists());
    Ok(())
}

#[cfg(not(unix))]
#[test]
fn non_unix_process_liveness_uses_age_only_fallback() {
    assert!(!process_alive(i64::from(std::process::id())));
}
