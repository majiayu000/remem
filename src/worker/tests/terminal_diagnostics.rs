use super::*;

#[test]
fn worker_terminal_failures_log_errors_without_retry_promises() -> anyhow::Result<()> {
    for (attempts, message, terminal) in [
        (0, "synthetic transient failure", false),
        (5, "synthetic exhausted failure", true),
        (0, "invalid payload: synthetic failure", true),
    ] {
        let data_dir = ScopedTestDataDir::new("worker-terminal-diagnostics");
        let mut conn = db::open_db()?;
        let job_id = enqueue_worker_job(&conn, db::JobType::Compress, "/synthetic")?;
        conn.execute(
            "UPDATE jobs SET attempt_count = ?1, max_attempts = 6 WHERE id = ?2",
            params![attempts, job_id],
        )?;
        db::claim_next_job(&mut conn, "worker-a", 60)?.expect("job should claim");
        record_failed_job_transition(
            &conn,
            job_id,
            db::JobType::Compress,
            "/synthetic",
            "worker-a",
            message,
            30,
        )?;
        let log = read_worker_log(&data_dir)?;
        let line = log
            .lines()
            .find(|line| line.contains(&format!("job id={job_id} failed")))
            .expect("failure diagnostic should be logged");
        if terminal {
            assert!(line.contains("ERROR"), "{line}");
            assert!(line.contains("no retry scheduled"), "{line}");
            assert!(!line.contains("retry in"), "{line}");
        } else {
            assert!(line.contains("WARN"), "{line}");
            assert!(line.contains("retry in 30s"), "{line}");
        }
    }
    Ok(())
}
