use anyhow::Result;

use super::super::current_state;
use super::support::*;

#[test]
fn g2_ineligible_active_row_is_not_returned_as_current() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        None,
        None,
    )?;
    mark_memory_legacy_unverified(&conn, 2)?;
    set_current_memory(&conn, 2)?;

    let result = current_state(&conn, &request())?;

    assert_eq!(result.status, "no_current");
    assert!(result.current.is_none());
    assert!(result.conflicts.is_empty());
    Ok(())
}
