use anyhow::Result;

use crate::db::test_support::ScopedTestDataDir;
use crate::memory::preference::render_preferences_with_context_details;

use super::{insert_preference_row, setup_test_db};

#[test]
fn preference_details_preserve_every_canonical_selection_drop_reason() -> Result<()> {
    let cwd = ScopedTestDataDir::new("preference-selection-audit");
    std::fs::create_dir_all(&cwd.path)?;
    std::fs::write(cwd.path.join("CLAUDE.md"), "Use Chinese comments\n")?;
    let conn = setup_test_db();
    insert_preference_row(
        &conn,
        1,
        "test/proj",
        Some("claude-dedup"),
        "Preference: Use Chinese comments",
        "Use Chinese comments in code",
        "project",
    )?;
    insert_preference_row(
        &conn,
        2,
        "test/proj",
        Some("format-duplicate"),
        "Preference: Format validation",
        "Always run cargo fmt before checks.",
        "project",
    )?;
    insert_preference_row(
        &conn,
        3,
        "test/proj",
        Some("docs"),
        "Preference: Keep docs current",
        "Update user documentation whenever behavior changes.",
        "project",
    )?;
    insert_preference_row(
        &conn,
        4,
        "test/proj",
        Some("format-primary"),
        "Preference: Format first",
        "Always run cargo fmt before checks.",
        "project",
    )?;

    let mut output = String::new();
    let details = render_preferences_with_context_details(
        &mut output,
        &conn,
        "test/proj",
        cwd.path.to_str().expect("utf-8 test path"),
        20,
        0,
        50,
    )?;
    let reasons = details
        .selection_drops
        .iter()
        .map(|drop| (drop.memory.id, drop.reason))
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(details.rendered_ids, vec![4]);
    assert_eq!(reasons.get(&1), Some(&"claude_md_dedup"));
    assert_eq!(reasons.get(&2), Some(&"preference_similarity_dedup"));
    assert_eq!(reasons.get(&3), Some(&"preference_char_limit"));
    Ok(())
}
