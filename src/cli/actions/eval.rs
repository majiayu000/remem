use anyhow::Result;

use crate::db;

pub(in crate::cli) fn run_eval_local() -> Result<()> {
    let conn = db::open_db()?;
    let report = crate::eval::local::run_eval(&conn)?;
    print!("{}", report);
    Ok(())
}

pub(in crate::cli) fn run_eval(dataset_path: &str, k: usize) -> Result<()> {
    let conn = db::open_db()?;
    let report = crate::eval::golden::run_dataset_path(&conn, dataset_path, k)?;
    print!("{}", report);
    Ok(())
}

pub(in crate::cli) async fn run_eval_e2e(k: usize, json: bool, keep_data_dir: bool) -> Result<()> {
    let report =
        crate::eval::e2e::run_sandbox_eval(crate::eval::e2e::E2eEvalOptions { k, keep_data_dir })
            .await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report);
    }
    Ok(())
}
