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
