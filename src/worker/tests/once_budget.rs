use crate::db::{self, test_support::ScopedTestDataDir};

use super::super::run;
use super::{configure_codex_stub, insert_compressible_observations};

#[tokio::test]
async fn worker_once_stops_after_four_work_items_across_the_real_loop() -> anyhow::Result<()> {
    let _data_dir = ScopedTestDataDir::new("worker-once-work-item-budget");
    configure_codex_stub("/tmp/remem-missing-codex-for-once-budget")?;
    let conn = db::open_db()?;
    for index in 0..6 {
        let project = format!("/tmp/remem-budget-{index}");
        insert_compressible_observations(&conn, &project, 101)?;
        db::enqueue_job(
            &conn,
            "codex-cli",
            db::JobType::Compress,
            &project,
            None,
            "{}",
            200,
        )?;
    }
    drop(conn);

    run(true, 10).await?;

    let conn = db::open_db()?;
    let (attempted, untouched): (i64, i64) = conn.query_row(
        "SELECT
             SUM(CASE WHEN attempt_count = 1 THEN 1 ELSE 0 END),
             SUM(CASE WHEN attempt_count = 0 THEN 1 ELSE 0 END)
         FROM jobs WHERE project LIKE '/tmp/remem-budget-%'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    anyhow::ensure!(
        attempted == 4,
        "once worker should attempt four AI jobs, got {attempted}"
    );
    anyhow::ensure!(
        untouched == 2,
        "once worker should leave two AI jobs for a later admission, got {untouched}"
    );
    Ok(())
}
