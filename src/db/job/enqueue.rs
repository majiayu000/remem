use anyhow::{bail, Context, Result};
use rusqlite::{
    params, Connection, Error as SqliteError, ErrorCode, OptionalExtension, Transaction,
    TransactionBehavior,
};

use crate::db::job::{dream_profile_key, JobType};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DreamEnqueueDecision {
    Enqueued(i64),
    CoalescedInflight(i64),
    SuppressedRecentDone(i64),
}

impl DreamEnqueueDecision {
    pub fn disposition(self) -> &'static str {
        match self {
            Self::Enqueued(_) => "enqueued",
            Self::CoalescedInflight(_) => "coalesced_inflight",
            Self::SuppressedRecentDone(_) => "suppressed_recent_done",
        }
    }

    pub fn job_id(self) -> i64 {
        match self {
            Self::Enqueued(id) | Self::CoalescedInflight(id) | Self::SuppressedRecentDone(id) => id,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveIdentity {
    Ordinary,
    Dream,
    CompileRules,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EnqueueCoreOutcome {
    Enqueued(i64),
    Coalesced(i64),
}

impl EnqueueCoreOutcome {
    fn job_id(self) -> i64 {
        match self {
            Self::Enqueued(id) | Self::Coalesced(id) => id,
        }
    }
}

pub fn enqueue_job(
    conn: &Connection,
    host: &str,
    job_type: JobType,
    project: &str,
    session_id: Option<&str>,
    payload_json: &str,
    priority: i64,
) -> Result<i64> {
    reject_summary(job_type)?;
    if !conn.is_autocommit() {
        return enqueue_job_in_transaction(
            conn,
            host,
            job_type,
            project,
            session_id,
            payload_json,
            priority,
        );
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .context("begin atomic job enqueue transaction")?;
    let outcome = enqueue_job_core(
        &tx,
        host,
        job_type,
        project,
        session_id,
        payload_json,
        priority,
    )?;
    tx.commit()
        .context("commit atomic job enqueue transaction")?;
    Ok(outcome.job_id())
}

pub(crate) fn enqueue_job_in_transaction(
    conn: &Connection,
    host: &str,
    job_type: JobType,
    project: &str,
    session_id: Option<&str>,
    payload_json: &str,
    priority: i64,
) -> Result<i64> {
    reject_summary(job_type)?;
    enqueue_job_core(
        conn,
        host,
        job_type,
        project,
        session_id,
        payload_json,
        priority,
    )
    .map(EnqueueCoreOutcome::job_id)
}

pub fn maybe_enqueue_dream_job(
    conn: &Connection,
    host: &str,
    project: &str,
    payload_json: &str,
    priority: i64,
    cooldown_secs: i64,
) -> Result<DreamEnqueueDecision> {
    if !conn.is_autocommit() {
        return maybe_enqueue_dream_job_in_transaction(
            conn,
            host,
            project,
            payload_json,
            priority,
            cooldown_secs,
        );
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .context("begin atomic Dream enqueue transaction")?;
    let decision =
        maybe_enqueue_dream_job_core(&tx, host, project, payload_json, priority, cooldown_secs)?;
    tx.commit()
        .context("commit atomic Dream enqueue transaction")?;
    Ok(decision)
}

pub(crate) fn maybe_enqueue_dream_job_in_transaction(
    conn: &Connection,
    host: &str,
    project: &str,
    payload_json: &str,
    priority: i64,
    cooldown_secs: i64,
) -> Result<DreamEnqueueDecision> {
    maybe_enqueue_dream_job_core(conn, host, project, payload_json, priority, cooldown_secs)
}

fn enqueue_job_core(
    conn: &Connection,
    host: &str,
    job_type: JobType,
    project: &str,
    session_id: Option<&str>,
    payload_json: &str,
    priority: i64,
) -> Result<EnqueueCoreOutcome> {
    reject_summary(job_type)?;
    let identity = active_identity(job_type);
    let normalized_session = match identity {
        ActiveIdentity::Ordinary => session_id,
        ActiveIdentity::Dream | ActiveIdentity::CompileRules => None,
    };
    if !test_skip_initial_identity_lookup() {
        if let Some(id) =
            find_active_canonical(conn, identity, host, job_type, project, normalized_session)?
        {
            return Ok(EnqueueCoreOutcome::Coalesced(id));
        }
    }

    let now = chrono::Utc::now().timestamp();
    let insert = conn.execute(
        "INSERT INTO jobs
         (host, job_type, project, session_id, payload_json, state, priority,
          attempt_count, max_attempts, lease_owner, lease_expires_epoch,
          next_retry_epoch, last_error, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, 0, 6, NULL, NULL, ?7, NULL, ?7, ?7)",
        params![
            host,
            job_type.as_str(),
            project,
            normalized_session,
            payload_json,
            priority,
            now
        ],
    );
    match insert {
        Ok(1) => Ok(EnqueueCoreOutcome::Enqueued(conn.last_insert_rowid())),
        Ok(count) => bail!("job enqueue invariant violated: inserted_rows={count}"),
        Err(error) if is_identity_unique_conflict(&error, identity) => {
            test_before_identity_conflict_reread(conn)?;
            let canonical =
                find_active_canonical(conn, identity, host, job_type, project, normalized_session)
                    .context("reread active canonical after job identity conflict")?;
            canonical.map(EnqueueCoreOutcome::Coalesced).ok_or_else(|| {
                anyhow::anyhow!(
                    "job identity conflict had no active canonical: identity={identity:?} job_type={} project={project}",
                    job_type.as_str()
                )
            })
        }
        Err(error) => Err(error.into()),
    }
}

fn maybe_enqueue_dream_job_core(
    conn: &Connection,
    host: &str,
    project: &str,
    payload_json: &str,
    priority: i64,
    cooldown_secs: i64,
) -> Result<DreamEnqueueDecision> {
    let incoming_profile = dream_profile_key(payload_json);
    let inflight: Option<(i64, String, String)> = conn
        .query_row(
            "SELECT id, state, payload_json FROM jobs
             WHERE job_type = 'dream' AND project = ?1
               AND state IN ('pending', 'processing')
             ORDER BY CASE state WHEN 'pending' THEN 0 ELSE 1 END,
                      updated_at_epoch DESC, id DESC
             LIMIT 1",
            params![project],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some((id, state, existing_payload)) = inflight {
        if state == "pending"
            && incoming_profile.is_some()
            && dream_profile_key(&existing_payload) != incoming_profile
        {
            let now = chrono::Utc::now().timestamp();
            let updated = conn.execute(
                "UPDATE jobs
                 SET host = ?1, payload_json = ?2,
                     priority = min(priority, ?3), updated_at_epoch = ?4
                 WHERE id = ?5 AND state = 'pending'",
                params![host, payload_json, priority, now, id],
            )?;
            if updated != 1 {
                bail!("pending Dream canonical changed before profile update: id={id}");
            }
        }
        return Ok(DreamEnqueueDecision::CoalescedInflight(id));
    }

    let now = chrono::Utc::now().timestamp();
    let cutoff = now - cooldown_secs.max(1);
    let recent_done: Option<i64> = conn
        .query_row(
            "SELECT id FROM jobs
             WHERE job_type = 'dream' AND project = ?1
               AND state = 'done' AND updated_at_epoch >= ?2
             ORDER BY updated_at_epoch DESC, id DESC LIMIT 1",
            params![project, cutoff],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = recent_done {
        return Ok(DreamEnqueueDecision::SuppressedRecentDone(id));
    }

    match enqueue_job_core(
        conn,
        host,
        JobType::Dream,
        project,
        None,
        payload_json,
        priority,
    )? {
        EnqueueCoreOutcome::Enqueued(id) => Ok(DreamEnqueueDecision::Enqueued(id)),
        EnqueueCoreOutcome::Coalesced(id) => Ok(DreamEnqueueDecision::CoalescedInflight(id)),
    }
}

fn reject_summary(job_type: JobType) -> Result<()> {
    if job_type == JobType::Summary {
        bail!("legacy Summary jobs are retired and cannot be enqueued");
    }
    Ok(())
}

fn active_identity(job_type: JobType) -> ActiveIdentity {
    match job_type {
        JobType::Dream => ActiveIdentity::Dream,
        JobType::CompileRules => ActiveIdentity::CompileRules,
        JobType::Observation | JobType::Summary | JobType::Compress => ActiveIdentity::Ordinary,
    }
}

fn find_active_canonical(
    conn: &Connection,
    identity: ActiveIdentity,
    host: &str,
    job_type: JobType,
    project: &str,
    session_id: Option<&str>,
) -> Result<Option<i64>> {
    let canonical = match identity {
        ActiveIdentity::Ordinary => conn
            .query_row(
                "SELECT id FROM jobs
                 WHERE host = ?1 AND job_type = ?2 AND project = ?3
                   AND COALESCE(session_id, '') = COALESCE(?4, '')
                   AND state IN ('pending', 'processing')
                 ORDER BY id ASC LIMIT 1",
                params![host, job_type.as_str(), project, session_id],
                |row| row.get(0),
            )
            .optional(),
        ActiveIdentity::Dream => conn
            .query_row(
                "SELECT id FROM jobs
                 WHERE job_type = 'dream' AND project = ?1
                   AND state IN ('pending', 'processing')
                 ORDER BY id ASC LIMIT 1",
                params![project],
                |row| row.get(0),
            )
            .optional(),
        ActiveIdentity::CompileRules => conn
            .query_row(
                "SELECT id FROM jobs
                 WHERE job_type = 'compile_rules' AND project = ?1 AND state = 'pending'
                 ORDER BY id ASC LIMIT 1",
                params![project],
                |row| row.get(0),
            )
            .optional(),
    };
    canonical.context("read active job canonical")
}

fn is_identity_unique_conflict(error: &SqliteError, identity: ActiveIdentity) -> bool {
    let SqliteError::SqliteFailure(code, message) = error else {
        return false;
    };
    if code.code != ErrorCode::ConstraintViolation {
        return false;
    }
    let message = message.as_deref().unwrap_or_default();
    match identity {
        ActiveIdentity::Ordinary => message.contains("idx_jobs_active_ordinary_unique"),
        ActiveIdentity::Dream => {
            message.contains("idx_jobs_active_dream_unique")
                || message.contains("UNIQUE constraint failed: jobs.project")
        }
        ActiveIdentity::CompileRules => {
            message.contains("idx_jobs_active_compile_rules_unique")
                || message.contains("UNIQUE constraint failed: jobs.project, jobs.state")
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum EnqueueTestFault {
    #[default]
    None,
    RereadActive,
    TerminalBeforeReread(i64),
    UnreadableBeforeReread,
}

#[cfg(test)]
thread_local! {
    static ENQUEUE_TEST_FAULT: std::cell::Cell<EnqueueTestFault> =
        const { std::cell::Cell::new(EnqueueTestFault::None) };
}

#[cfg(test)]
fn set_enqueue_test_fault(fault: EnqueueTestFault) {
    ENQUEUE_TEST_FAULT.with(|slot| slot.set(fault));
}

#[cfg(test)]
fn test_skip_initial_identity_lookup() -> bool {
    ENQUEUE_TEST_FAULT.with(|slot| slot.get() != EnqueueTestFault::None)
}

#[cfg(not(test))]
fn test_skip_initial_identity_lookup() -> bool {
    false
}

#[cfg(test)]
fn test_before_identity_conflict_reread(conn: &Connection) -> Result<()> {
    ENQUEUE_TEST_FAULT.with(|slot| match slot.replace(EnqueueTestFault::None) {
        EnqueueTestFault::TerminalBeforeReread(id) => {
            conn.execute("UPDATE jobs SET state = 'done' WHERE id = ?1", params![id])?;
            Ok(())
        }
        EnqueueTestFault::UnreadableBeforeReread => {
            bail!("injected canonical reread failure")
        }
        EnqueueTestFault::None | EnqueueTestFault::RereadActive => Ok(()),
    })
}

#[cfg(not(test))]
fn test_before_identity_conflict_reread(_conn: &Connection) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Barrier};
    use std::thread;

    use rusqlite::{params, Connection};

    use super::{
        enqueue_job, maybe_enqueue_dream_job, set_enqueue_test_fault, DreamEnqueueDecision,
        EnqueueTestFault,
    };
    use crate::db::job::JobType;
    use crate::db::test_support::{cleanup_temp_db_files, unique_temp_db_path};
    use crate::migrate::{run_migrations, MIGRATIONS};

    struct WalDb(PathBuf);

    impl WalDb {
        fn new(label: &str) -> Self {
            let path = unique_temp_db_path(label);
            let conn = Self::open_path(&path);
            run_migrations(&conn).expect("WAL test schema should migrate");
            drop(conn);
            Self(path)
        }

        fn open(&self) -> Connection {
            Self::open_path(&self.0)
        }

        fn open_path(path: &Path) -> Connection {
            let conn = Connection::open(path).expect("WAL test database should open");
            conn.pragma_update(None, "journal_mode", "WAL")
                .expect("WAL mode should enable");
            conn.pragma_update(None, "foreign_keys", "ON")
                .expect("foreign keys should enable");
            conn.busy_timeout(std::time::Duration::from_secs(30))
                .expect("busy timeout should configure");
            conn
        }
    }

    impl Drop for WalDb {
        fn drop(&mut self) {
            cleanup_temp_db_files(&self.0);
        }
    }

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        for migration in MIGRATIONS {
            conn.execute_batch(migration.sql)
                .expect("schema migration should load");
        }
        conn
    }

    fn ordinary_count(conn: &Connection, project: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE project = ?1",
            params![project],
            |row| row.get(0),
        )
        .expect("ordinary count should load")
    }

    #[test]
    fn enqueue_job_two_wal_connections_coalesce_ordinary_identity() {
        let db = WalDb::new("job-enqueue-ordinary");
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = [None, Some("")]
            .into_iter()
            .map(|session_id| {
                let path = db.0.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let conn = WalDb::open_path(&path);
                    barrier.wait();
                    enqueue_job(
                        &conn,
                        "codex-cli",
                        JobType::Compress,
                        "ordinary-wal",
                        session_id,
                        "{}",
                        100,
                    )
                    .expect("concurrent ordinary enqueue should succeed")
                })
            })
            .collect();
        let ids: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().expect("enqueue thread should join"))
            .collect();

        assert_eq!(ids[0], ids[1]);
        assert_eq!(ordinary_count(&db.open(), "ordinary-wal"), 1);
    }

    #[test]
    fn dream_two_wal_connections_coalesce_across_hosts() {
        let db = WalDb::new("job-enqueue-dream");
        let barrier = Arc::new(Barrier::new(2));
        let inputs = [
            ("codex-cli", "{}", 300),
            ("claude-code", r#"{"remem_ai_profile":"quality"}"#, 100),
        ];
        let handles: Vec<_> = inputs
            .into_iter()
            .map(|(host, payload, priority)| {
                let path = db.0.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let conn = WalDb::open_path(&path);
                    barrier.wait();
                    maybe_enqueue_dream_job(&conn, host, "dream-wal", payload, priority, 60)
                        .expect("concurrent Dream enqueue should succeed")
                })
            })
            .collect();
        let decisions: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().expect("Dream thread should join"))
            .collect();

        assert_eq!(decisions[0].job_id(), decisions[1].job_id());
        assert!(decisions
            .iter()
            .any(|decision| matches!(decision, DreamEnqueueDecision::CoalescedInflight(_))));
        let row: (i64, String, String, i64) = db
            .open()
            .query_row(
                "SELECT COUNT(*), host, payload_json, priority FROM jobs
                 WHERE job_type = 'dream' AND project = 'dream-wal'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("Dream canonical should load");
        assert_eq!(row, (1, "claude-code".into(), inputs[1].1.into(), 100));
    }

    fn concurrent_compile_rules(db: &WalDb, project: &'static str) -> Vec<i64> {
        let barrier = Arc::new(Barrier::new(2));
        ["codex-cli", "claude-code"]
            .into_iter()
            .map(|host| {
                let path = db.0.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let conn = WalDb::open_path(&path);
                    barrier.wait();
                    enqueue_job(
                        &conn,
                        host,
                        JobType::CompileRules,
                        project,
                        Some("ignored"),
                        "{}",
                        100,
                    )
                    .expect("concurrent CompileRules enqueue should succeed")
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().expect("CompileRules thread should join"))
            .collect()
    }

    #[test]
    fn compile_rules_two_wal_connections_share_one_pending_successor() {
        let db = WalDb::new("job-enqueue-compile-successor");
        let conn = db.open();
        let predecessor = enqueue_job(
            &conn,
            "worker",
            JobType::CompileRules,
            "compile-successor",
            None,
            "{}",
            100,
        )
        .expect("predecessor should enqueue");
        conn.execute(
            "UPDATE jobs SET state = 'processing', lease_owner = 'worker-a',
                 lease_expires_epoch = ?2 WHERE id = ?1",
            params![predecessor, chrono::Utc::now().timestamp() + 60],
        )
        .expect("predecessor should enter processing");
        drop(conn);

        let ids = concurrent_compile_rules(&db, "compile-successor");
        assert_eq!(ids[0], ids[1]);
        let states: (i64, i64) = db
            .open()
            .query_row(
                "SELECT SUM(state = 'processing'), SUM(state = 'pending') FROM jobs
                 WHERE job_type = 'compile_rules' AND project = 'compile-successor'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("CompileRules states should load");
        assert_eq!(states, (1, 1));
    }

    #[test]
    fn compile_rules_two_wal_connections_create_one_initial_pending() {
        let db = WalDb::new("job-enqueue-compile-initial");
        let ids = concurrent_compile_rules(&db, "compile-initial");
        assert_eq!(ids[0], ids[1]);
        assert_eq!(ordinary_count(&db.open(), "compile-initial"), 1);
    }

    #[test]
    fn ordinary_job_identity_normalizes_null_session_and_allows_terminal_history() {
        let conn = setup_conn();
        let first = enqueue_job(
            &conn,
            "codex-cli",
            JobType::Compress,
            "ordinary-history",
            None,
            "{}",
            100,
        )
        .expect("NULL-session job should enqueue");
        let duplicate = enqueue_job(
            &conn,
            "codex-cli",
            JobType::Compress,
            "ordinary-history",
            Some(""),
            "{}",
            100,
        )
        .expect("empty-session job should coalesce");
        assert_eq!(duplicate, first);
        conn.execute(
            "UPDATE jobs SET state = 'done' WHERE id = ?1",
            params![first],
        )
        .expect("canonical should become terminal");
        let successor = enqueue_job(
            &conn,
            "codex-cli",
            JobType::Compress,
            "ordinary-history",
            None,
            "{}",
            100,
        )
        .expect("terminal history should allow a successor");
        assert_ne!(successor, first);
        assert_eq!(ordinary_count(&conn, "ordinary-history"), 2);
    }

    fn canonical_fixture(conn: &Connection, project: &str) -> i64 {
        enqueue_job(
            conn,
            "codex-cli",
            JobType::Compress,
            project,
            None,
            "{}",
            100,
        )
        .expect("canonical fixture should enqueue")
    }

    fn retry_fixture_enqueue(conn: &Connection, project: &str) -> anyhow::Result<i64> {
        enqueue_job(
            conn,
            "codex-cli",
            JobType::Compress,
            project,
            Some(""),
            "{}",
            100,
        )
    }

    #[test]
    fn enqueue_job_identity_conflict_rereads_only_active_canonical() {
        let conn = setup_conn();
        let canonical = canonical_fixture(&conn, "conflict-active");
        set_enqueue_test_fault(EnqueueTestFault::RereadActive);
        let returned = retry_fixture_enqueue(&conn, "conflict-active")
            .expect("active canonical reread should coalesce");
        assert_eq!(returned, canonical);
        assert_eq!(ordinary_count(&conn, "conflict-active"), 1);
    }

    #[test]
    fn enqueue_job_identity_conflict_terminal_canonical_rolls_back() {
        let conn = setup_conn();
        let canonical = canonical_fixture(&conn, "conflict-terminal");
        set_enqueue_test_fault(EnqueueTestFault::TerminalBeforeReread(canonical));
        let error = retry_fixture_enqueue(&conn, "conflict-terminal")
            .expect_err("terminal canonical must fail closed");
        assert!(error.to_string().contains("no active canonical"));
        let state: String = conn
            .query_row(
                "SELECT state FROM jobs WHERE id = ?1",
                params![canonical],
                |row| row.get(0),
            )
            .expect("rolled-back canonical should load");
        assert_eq!(state, "pending");
    }

    #[test]
    fn enqueue_job_identity_conflict_unreadable_canonical_rolls_back() {
        let conn = setup_conn();
        canonical_fixture(&conn, "conflict-unreadable");
        set_enqueue_test_fault(EnqueueTestFault::UnreadableBeforeReread);
        let error = retry_fixture_enqueue(&conn, "conflict-unreadable")
            .expect_err("unreadable canonical must fail closed");
        assert!(error
            .to_string()
            .contains("injected canonical reread failure"));
        assert_eq!(ordinary_count(&conn, "conflict-unreadable"), 1);
    }

    #[test]
    fn enqueue_job_propagates_non_identity_constraint_errors() {
        let conn = setup_conn();
        conn.execute_batch(
            "CREATE TRIGGER reject_test_job BEFORE INSERT ON jobs BEGIN
                 SELECT RAISE(ABORT, 'injected nonidentity constraint');
             END;",
        )
        .expect("constraint trigger should install");
        let error = retry_fixture_enqueue(&conn, "constraint-error")
            .expect_err("non-identity constraint must propagate");
        assert!(format!("{error:#}").contains("injected nonidentity constraint"));
        assert_eq!(ordinary_count(&conn, "constraint-error"), 0);
    }

    #[test]
    fn enqueue_job_rejects_legacy_summary() {
        let conn = setup_conn();
        let error = enqueue_job(
            &conn,
            "codex-cli",
            JobType::Summary,
            "summary-retired",
            None,
            "{}",
            100,
        )
        .expect_err("legacy Summary enqueue must fail closed");
        assert!(error.to_string().contains("retired"));
        assert_eq!(ordinary_count(&conn, "summary-retired"), 0);
    }
}
