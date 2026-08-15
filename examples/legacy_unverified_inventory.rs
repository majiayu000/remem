//! Read-only aggregate G2 inventory. Requires an explicit reference epoch.
use anyhow::{bail, Context, Result};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let flag = args.next();
    let value = args.next();
    if flag.as_deref() != Some("--as-of-epoch") || value.is_none() || args.next().is_some() {
        bail!("usage: legacy_unverified_inventory --as-of-epoch <unix-epoch>");
    }
    let as_of_epoch = value
        .expect("checked")
        .parse::<i64>()
        .context("--as-of-epoch must be an integer")?;
    let conn = remem::db::open_db_read_only_current()
        .context("open current-schema remem database read-only")?;
    let report = remem::truth::build_memory_visibility_inventory(&conn, as_of_epoch)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
