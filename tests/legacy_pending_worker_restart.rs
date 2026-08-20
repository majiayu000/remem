#![cfg(unix)]

use fs2::FileExt;
use rusqlite::{params, types::Value, Connection};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const PROCESS_READY_TIMEOUT: Duration = Duration::from_secs(10);
const PROCESS_EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(5);

static NEXT_SANDBOX: AtomicU64 = AtomicU64::new(0);

struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    data_dir: PathBuf,
    config_path: PathBuf,
    bin_dir: PathBuf,
    tmp_dir: PathBuf,
    project_dir: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let sequence = NEXT_SANDBOX.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "remem-legacy-worker-restart-{}-{sequence}-{nanos}",
            std::process::id()
        ));
        let home = root.join("home");
        let data_dir = root.join("data");
        let bin_dir = root.join("bin");
        let tmp_dir = root.join("tmp");
        let project_dir = root.join("project");
        for dir in [&home, &data_dir, &bin_dir, &tmp_dir, &project_dir] {
            fs::create_dir_all(dir).expect("create isolated test directory");
        }
        Self {
            config_path: data_dir.join("config.toml"),
            root,
            home,
            data_dir,
            bin_dir,
            tmp_dir,
            project_dir,
        }
    }

    fn isolated_remem_command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_remem"));
        command
            .current_dir(&self.project_dir)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("REMEM_DATA_DIR", &self.data_dir)
            .env("REMEM_CONFIG", &self.config_path)
            .env("REMEM_ALLOW_PLAINTEXT_DB", "1")
            .env("TMPDIR", &self.tmp_dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("NO_COLOR", "1");
        for name in "REMEM_CIPHER_KEY REMEM_DISABLE_HOOKS REMEM_DEBUG REMEM_EMBEDDINGS_API_KEY REMEM_EMBEDDING_API_KEY REMEM_EMBEDDINGS_API_KEY_ENV REMEM_STDERR_TO_LOG".split_whitespace() {
            command.env_remove(name);
        }
        command
    }

    fn isolated_database_path(&self) -> PathBuf {
        self.data_dir.join("remem.db")
    }

    fn worker_lock_path(&self) -> PathBuf {
        self.data_dir.join("worker.lock")
    }

    fn install_codex_stub(&self) -> PathBuf {
        let path = self.bin_dir.join("codex-stub.sh");
        let script = r#"#!/bin/sh
prev=""
output_path=""
for arg in "$@"; do
  [ "$prev" = "--output-last-message" ] && { output_path="$arg"; break; }
  prev="$arg"
done
[ -n "$output_path" ] || { echo "missing output path" >&2; exit 1; }
stdin_path="${TMPDIR:-/tmp}/remem-restart-codex-$$.txt"
cat > "$stdin_path"
if grep -q "Task: memory_candidate" "$stdin_path"; then
  printf '%s\n' '<memory_candidate><scope>project</scope><type>decision</type><topic_key>process-worker-restart</topic_key><risk_class>low</risk_class><confidence>0.91</confidence><text>Process-level worker restart recovered the legacy observation.</text></memory_candidate>' > "$output_path"
  rm -f "$stdin_path"
  exit 0
fi
if grep -q "Task: graph_candidate" "$stdin_path"; then
  printf '%s\n' '<no_graph_candidates reason="restart stub has no graph facts"/>' > "$output_path"
  rm -f "$stdin_path"
  exit 0
fi
printf '%s\n' '{"observations":[{"type":"decision","title":"Process-level worker restart","subtitle":null,"narrative":"Process-level worker restart recovered the legacy observation.","facts":[],"concepts":[],"files_read":[],"files_modified":[],"confidence":0.9}]}' > "$output_path"
rm -f "$stdin_path"
"#;
        write_executable(&path, script);
        path
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct ProcessOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

struct LoggedChild {
    child: Option<Child>,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

impl LoggedChild {
    fn spawn_logged_remem(mut command: Command, sandbox: &Sandbox, label: &str) -> Self {
        let stdout_path = sandbox.root.join(format!("{label}.stdout"));
        let stderr_path = sandbox.root.join(format!("{label}.stderr"));
        command
            .stdout(Stdio::from(
                File::create(&stdout_path).expect("create child stdout file"),
            ))
            .stderr(Stdio::from(
                File::create(&stderr_path).expect("create child stderr file"),
            ));
        let child = command.spawn().expect("spawn remem CLI process");
        Self {
            child: Some(child),
            stdout_path,
            stderr_path,
        }
    }

    fn id(&self) -> u32 {
        self.child.as_ref().expect("child should be live").id()
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child
            .as_mut()
            .expect("child should be live")
            .try_wait()
    }

    fn logs(&self) -> (String, String) {
        (
            fs::read_to_string(&self.stdout_path).unwrap_or_default(),
            fs::read_to_string(&self.stderr_path).unwrap_or_default(),
        )
    }

    fn wait(mut self, timeout: Duration) -> ProcessOutput {
        let deadline = Instant::now() + timeout;
        let mut timed_out = false;
        let status = loop {
            match self.try_wait().expect("poll remem CLI process") {
                Some(status) => break status,
                None if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
                None => {
                    timed_out = true;
                    signal_pid(self.id(), libc::SIGKILL)
                        .expect("terminate timed-out remem CLI process");
                    break self
                        .child
                        .as_mut()
                        .expect("child should be live")
                        .wait()
                        .expect("reap timed-out remem CLI process");
                }
            }
        };
        self.child.take();
        let (stdout, stderr) = self.logs();
        ProcessOutput {
            status,
            stdout,
            stderr,
            timed_out,
        }
    }
}

impl Drop for LoggedChild {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        if child.try_wait().ok().flatten().is_none() {
            let _ = signal_pid(child.id(), libc::SIGKILL);
            let _ = child.wait();
        }
    }
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write executable test stub");
    let mut permissions = fs::metadata(path)
        .expect("read executable test stub metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("mark test stub executable");
}

fn signal_pid(process_id: u32, signal: i32) -> io::Result<()> {
    let result = unsafe { libc::kill(process_id as libc::pid_t, signal) };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(error)
}

fn run_success(command: Command, sandbox: &Sandbox, label: &str) -> ProcessOutput {
    let output = LoggedChild::spawn_logged_remem(command, sandbox, label).wait(COMMAND_TIMEOUT);
    assert!(
        !output.timed_out && output.status.success(),
        "{label} failed (timed_out={}, status={})\nstdout:\n{}\nstderr:\n{}",
        output.timed_out,
        output.status,
        output.stdout,
        output.stderr
    );
    output
}

fn initialize_database(sandbox: &Sandbox, codex_path: Option<&Path>) {
    let mut config_init = sandbox.isolated_remem_command();
    config_init.args(["config", "init"]);
    run_success(config_init, sandbox, "config-init");
    if let Some(codex_path) = codex_path {
        let mut config_set_path = sandbox.isolated_remem_command();
        config_set_path
            .args(["config", "set", "memory_ai.profiles.codex.path"])
            .arg(codex_path);
        run_success(config_set_path, sandbox, "config-set-codex-path");
    }
    let mut disable_rule_sweep = sandbox.isolated_remem_command();
    disable_rule_sweep.args(["config", "set", "rule_compilation.enabled", "false"]);
    run_success(disable_rule_sweep, sandbox, "config-disable-rule-sweep");
    let mut initialize_db = sandbox.isolated_remem_command();
    initialize_db.args(["worker", "--once"]);
    run_success(initialize_db, sandbox, "initialize-database");
}

fn wait_for_worker_start(worker: &mut LoggedChild) {
    let deadline = Instant::now() + PROCESS_READY_TIMEOUT;
    loop {
        let (_, stderr) = worker.logs();
        if stderr.contains("[INFO] [worker] start ") {
            return;
        }
        if let Some(status) = worker.try_wait().expect("poll in-flight worker") {
            let (stdout, stderr) = worker.logs();
            panic!(
                "worker exited before publishing its start log: \
                 status={status}\nstdout:\n{stdout}\nstderr:\n{stderr}"
            );
        }
        if Instant::now() >= deadline {
            let (stdout, stderr) = worker.logs();
            panic!(
                "worker did not publish its start log within {PROCESS_READY_TIMEOUT:?}\n\
                 stdout:\n{stdout}\nstderr:\n{stderr}"
            );
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn assert_worker_lock_reacquirable(path: &Path) {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .expect("open worker singleton lock");
    FileExt::try_lock_exclusive(&file).expect("worker singleton lock should be reacquirable");
    FileExt::unlock(&file).expect("release worker singleton lock probe");
}

fn assert_worker_lock_held(path: &Path) {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .expect("open worker singleton lock");
    match FileExt::try_lock_exclusive(&file) {
        Ok(()) => {
            FileExt::unlock(&file).expect("release unexpected worker lock acquisition");
            panic!("live worker should hold its singleton lock");
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
        Err(error) => panic!("probe live worker singleton lock: {error}"),
    }
}

fn seed_not_due_failed_legacy_backlog(sandbox: &Sandbox) -> (i64, i64) {
    let conn = open_database(sandbox);
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")
        .expect("configure fixture database connection");
    let now = chrono::Utc::now().timestamp();
    let next_retry_epoch = now + 3_600;
    conn.execute(
        "INSERT INTO pending_observations
         (host, session_id, project, tool_name, tool_input, tool_response, cwd,
          created_at_epoch, updated_at_epoch, status, attempt_count,
          next_retry_epoch, last_error, lease_owner, lease_expires_epoch,
          failure_class, failed_at_epoch, archived_at_epoch)
         VALUES (?1, ?2, ?3, 'Bash', ?4, ?5, ?6, ?7, ?8, 'failed', 3,
                 ?9, 'not due for automatic recovery', NULL, NULL,
                 'transient', ?10, NULL)",
        params![
            "codex-cli",
            "process-worker-restart-session",
            sandbox.project_dir.to_string_lossy().as_ref(),
            r#"{"cmd":"printf legacy"}"#,
            r#"{"output":"legacy"}"#,
            sandbox.project_dir.to_string_lossy().as_ref(),
            now - 3_600,
            now - 600,
            next_retry_epoch,
            now - 600,
        ],
    )
    .expect("seed legacy pending observation");
    let pending_id = conn.last_insert_rowid();
    remem::db::pending::admin::reactivate_legacy_pending_bridge(&conn)
        .expect("mark the synthetic historical backlog as frozen-draining");
    (pending_id, next_retry_epoch)
}

fn open_database(sandbox: &Sandbox) -> Connection {
    Connection::open(sandbox.isolated_database_path()).expect("open isolated remem database")
}

fn source_recovery_state(
    conn: &Connection,
    pending_id: i64,
) -> (String, i64, Option<i64>, Option<i64>) {
    conn.query_row(
        "SELECT status, attempt_count, next_retry_epoch, archived_at_epoch
         FROM pending_observations WHERE id = ?1",
        [pending_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .expect("read legacy recovery state")
}

fn source_snapshot(conn: &Connection, pending_id: i64) -> Vec<Value> {
    let mut statement = conn
        .prepare(
            "SELECT id, host, session_id, project, tool_name, tool_input, tool_response, cwd,
                    created_at_epoch, updated_at_epoch, status, attempt_count, next_retry_epoch,
                    last_error, lease_owner, lease_expires_epoch, failure_class, failed_at_epoch,
                    archived_at_epoch
             FROM pending_observations
             WHERE id = ?1",
        )
        .expect("prepare full legacy source snapshot");
    let column_count = statement.column_count();
    statement
        .query_row([pending_id], |row| {
            (0..column_count)
                .map(|index| row.get(index))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .expect("read full legacy source snapshot")
}

fn captured_event_count(conn: &Connection, pending_id: i64) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM captured_events WHERE event_id = ?1",
        [format!("legacy-pending-{pending_id}")],
        |row| row.get(0),
    )
    .expect("count captured legacy events")
}

fn count_rows(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0))
        .expect("count database rows")
}

#[test]
fn sigkill_after_worker_start_preserves_not_due_backlog_and_once_restart_recovers_it() {
    let sandbox = Sandbox::new();
    let codex_stub = sandbox.install_codex_stub();
    initialize_database(&sandbox, Some(&codex_stub));
    let (pending_id, next_retry_epoch) = seed_not_due_failed_legacy_backlog(&sandbox);
    let before_snapshot = {
        let conn = open_database(&sandbox);
        let state = source_recovery_state(&conn, pending_id);
        assert_eq!(
            state,
            ("failed".to_string(), 3, Some(next_retry_epoch), None)
        );
        assert_eq!(captured_event_count(&conn, pending_id), 0);
        source_snapshot(&conn, pending_id)
    };

    let mut daemon_command = sandbox.isolated_remem_command();
    daemon_command.arg("worker");
    let mut daemon =
        LoggedChild::spawn_logged_remem(daemon_command, &sandbox, "idle-worker-daemon");
    wait_for_worker_start(&mut daemon);
    assert_worker_lock_held(&sandbox.worker_lock_path());
    signal_pid(daemon.id(), libc::SIGKILL).expect("SIGKILL started worker process");
    let killed = daemon.wait(PROCESS_EXIT_TIMEOUT);
    assert!(!killed.timed_out, "SIGKILLed worker should exit promptly");
    assert_eq!(
        killed.status.signal(),
        Some(libc::SIGKILL),
        "started worker should terminate via SIGKILL\nstdout:\n{}\nstderr:\n{}",
        killed.stdout,
        killed.stderr
    );

    let conn = open_database(&sandbox);
    assert_eq!(
        source_snapshot(&conn, pending_id),
        before_snapshot,
        "SIGKILL after worker start must not mutate any field of a not-due source row"
    );
    assert_eq!(
        captured_event_count(&conn, pending_id),
        0,
        "SIGKILL must not create a current-pipeline event"
    );
    drop(conn);
    assert_worker_lock_reacquirable(&sandbox.worker_lock_path());

    let due_at = chrono::Utc::now().timestamp() - 1;
    let conn = open_database(&sandbox);
    conn.execute(
        "UPDATE pending_observations
         SET next_retry_epoch = ?2, updated_at_epoch = ?2
         WHERE id = ?1",
        params![pending_id, due_at],
    )
    .expect("make failed legacy backlog due for restart recovery");
    drop(conn);

    let mut restart = sandbox.isolated_remem_command();
    restart.args(["worker", "--once"]);
    run_success(restart, &sandbox, "restart-worker-once");

    let conn = open_database(&sandbox);
    let failed_backlog = count_rows(
        &conn,
        "SELECT COUNT(*) FROM pending_observations WHERE status = 'failed'",
    );
    assert_eq!(failed_backlog, 0, "worker restart should drain the backlog");
    let recovered_source = source_recovery_state(&conn, pending_id);
    assert_eq!(recovered_source, ("migrated".to_string(), 0, None, None));
    let captured_after_restart = captured_event_count(&conn, pending_id);
    assert_eq!(captured_after_restart, 1);
    let recovered_observations = count_rows(
        &conn,
        "SELECT COUNT(*) FROM observations
         WHERE text LIKE '%Process-level worker restart recovered%'",
    );
    assert!(
        recovered_observations >= 1,
        "restart should process the current ObservationExtract task"
    );
    let completed_extraction_tasks = count_rows(
        &conn,
        "SELECT COUNT(*) FROM extraction_tasks
         WHERE task_kind = 'observation_extract' AND status = 'done'",
    );
    assert_eq!(completed_extraction_tasks, 1);
    drop(conn);
    assert_worker_lock_reacquirable(&sandbox.worker_lock_path());
}
