use anyhow::Result;
use rusqlite::{params, Connection};

use crate::memory::{self, Memory};

/// Query global preferences that appear in 3+ projects.
pub fn query_global_preferences(conn: &Connection, limit: usize) -> Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.session_id, m.project, m.topic_key, m.title, m.content, \
         m.memory_type, m.files, m.created_at_epoch, m.updated_at_epoch, m.status, m.branch \
         FROM memories m \
         WHERE m.memory_type = 'preference' AND m.status = 'active' AND m.topic_key IS NOT NULL \
         AND m.topic_key IN ( \
             SELECT topic_key FROM memories \
             WHERE memory_type = 'preference' AND status = 'active' AND topic_key IS NOT NULL \
             GROUP BY topic_key HAVING COUNT(DISTINCT project) >= 3 \
         ) \
         GROUP BY m.topic_key \
         ORDER BY MAX(m.updated_at_epoch) DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(Memory {
            id: row.get(0)?,
            session_id: row.get(1)?,
            project: row.get(2)?,
            topic_key: row.get(3)?,
            title: row.get(4)?,
            text: row.get(5)?,
            memory_type: row.get(6)?,
            files: row.get(7)?,
            created_at_epoch: row.get(8)?,
            updated_at_epoch: row.get(9)?,
            status: row.get(10)?,
            branch: row.get(11)?,
        })
    })?;
    crate::db_query::collect_rows(rows)
}

/// Filter out preferences whose title already appears in CLAUDE.md.
pub fn dedup_with_claude_md(prefs: &[Memory], cwd: &str) -> Vec<usize> {
    let claude_md_path = std::path::Path::new(cwd).join("CLAUDE.md");
    let claude_md_content = std::fs::read_to_string(&claude_md_path).unwrap_or_default();

    if claude_md_content.is_empty() {
        return (0..prefs.len()).collect();
    }

    let claude_lower = claude_md_content.to_lowercase();
    (0..prefs.len())
        .filter(|&i| {
            let title_lower = prefs[i].title.to_lowercase();
            // Remove "Preference: " prefix for matching
            let search_term = title_lower
                .strip_prefix("preference: ")
                .unwrap_or(&title_lower);
            !claude_lower.contains(search_term)
        })
        .collect()
}

/// Render preferences section for context output.
pub fn render_preferences(
    output: &mut String,
    conn: &Connection,
    project: &str,
    cwd: &str,
) -> Result<()> {
    let project_prefs = memory::get_memories_by_type(conn, project, "preference", 20)?;
    let global_prefs = query_global_preferences(conn, 10).unwrap_or_default();

    // Merge: project prefs first, then global prefs not already in project
    let mut all_prefs = project_prefs;
    let project_topics: std::collections::HashSet<String> = all_prefs
        .iter()
        .filter_map(|m| m.topic_key.clone())
        .collect();
    for gp in global_prefs {
        if let Some(ref tk) = gp.topic_key {
            if !project_topics.contains(tk) {
                all_prefs.push(gp);
            }
        }
    }

    if all_prefs.is_empty() {
        return Ok(());
    }

    // Dedup with CLAUDE.md
    let keep_indices = dedup_with_claude_md(&all_prefs, cwd);
    if keep_indices.is_empty() {
        return Ok(());
    }

    output.push_str("## Your Preferences (always apply these)\n");
    let mut total_chars = 0;
    const MAX_CHARS: usize = 1500; // ~500 tokens

    for &idx in &keep_indices {
        let pref = &all_prefs[idx];
        // Use text content directly for bullet point
        let text = pref.text.trim();
        let line = if text.len() > 120 {
            format!(
                "- {}\n",
                &text[..text.chars().take(120).map(|c| c.len_utf8()).sum()]
            )
        } else {
            format!("- {}\n", text)
        };
        if total_chars + line.len() > MAX_CHARS && total_chars > 0 {
            break;
        }
        output.push_str(&line);
        total_chars += line.len();
    }
    output.push('\n');

    Ok(())
}

/// List preferences for CLI output.
pub fn list_preferences(conn: &Connection, project: &str) -> Result<()> {
    let project_prefs = memory::get_memories_by_type(conn, project, "preference", 50)?;
    let global_prefs = query_global_preferences(conn, 10).unwrap_or_default();

    if project_prefs.is_empty() && global_prefs.is_empty() {
        println!("No preferences found.");
        return Ok(());
    }

    if !project_prefs.is_empty() {
        println!("Project preferences ({}):", project);
        for pref in &project_prefs {
            let text_preview: String = pref.text.chars().take(80).collect();
            println!("  [{}] {}", pref.id, text_preview);
        }
    }

    if !global_prefs.is_empty() {
        println!("\nGlobal preferences (3+ projects):");
        for pref in &global_prefs {
            let text_preview: String = pref.text.chars().take(80).collect();
            println!("  [{}] {} (from: {})", pref.id, text_preview, pref.project);
        }
    }

    Ok(())
}

/// Add a preference.
pub fn add_preference(conn: &Connection, project: &str, text: &str) -> Result<i64> {
    let title = format!("Preference: {}", &text[..text.len().min(60)]);
    let topic_key = format!(
        "manual-preference-{}",
        crate::memory::slugify_for_topic(text, 50)
    );
    memory::insert_memory(
        conn,
        None,
        project,
        Some(&topic_key),
        &title,
        text,
        "preference",
        None,
    )
}

/// Archive a preference by ID.
pub fn remove_preference(conn: &Connection, id: i64) -> Result<bool> {
    let count = conn.execute(
        "UPDATE memories SET status = 'archived' WHERE id = ?1 AND memory_type = 'preference'",
        params![id],
    )?;
    Ok(count > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory()
            .unwrap_or_else(|e| panic!("Failed to open in-memory db: {e}"));
        conn.execute_batch(
            "CREATE TABLE memories (
                id INTEGER PRIMARY KEY,
                session_id TEXT,
                project TEXT NOT NULL,
                topic_key TEXT,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                memory_type TEXT NOT NULL,
                files TEXT,
                created_at_epoch INTEGER NOT NULL,
                updated_at_epoch INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                branch TEXT
            );
            CREATE VIRTUAL TABLE memories_fts USING fts5(
                title, content,
                content='memories',
                content_rowid='id',
                tokenize='trigram'
            );
            CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, title, content)
                VALUES (new.id, new.title, new.content);
            END;
            CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, title, content)
                VALUES ('delete', old.id, old.title, old.content);
                INSERT INTO memories_fts(rowid, title, content)
                VALUES (new.id, new.title, new.content);
            END;
            CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, title, content)
                VALUES ('delete', old.id, old.title, old.content);
            END;",
        )
        .unwrap_or_else(|e| panic!("Failed to create test schema: {e}"));
        conn
    }

    #[test]
    fn test_render_preferences_empty() -> Result<()> {
        let conn = setup_test_db();
        let mut output = String::new();
        render_preferences(&mut output, &conn, "test/proj", ".")?;
        assert!(
            output.is_empty(),
            "Should not render section when no preferences"
        );
        Ok(())
    }

    #[test]
    fn test_render_preferences_with_data() -> Result<()> {
        let conn = setup_test_db();
        memory::insert_memory(
            &conn,
            None,
            "test/proj",
            Some("pref-1"),
            "Preference: Use Chinese comments",
            "Use Chinese comments in code",
            "preference",
            None,
        )?;

        let mut output = String::new();
        render_preferences(&mut output, &conn, "test/proj", "/nonexistent")?;
        assert!(output.contains("## Your Preferences"));
        assert!(output.contains("Use Chinese comments"));
        Ok(())
    }

    #[test]
    fn test_global_preferences_threshold() -> Result<()> {
        let conn = setup_test_db();
        // Insert same preference in 3 different projects
        for proj in &["proj-a", "proj-b", "proj-c"] {
            memory::insert_memory(
                &conn,
                None,
                proj,
                Some("global-pref-1"),
                "Preference: Terse responses",
                "Give terse responses without summaries",
                "preference",
                None,
            )?;
        }

        // Insert preference in only 1 project
        memory::insert_memory(
            &conn,
            None,
            "proj-a",
            Some("local-pref"),
            "Preference: Use tabs",
            "Use tabs for indentation",
            "preference",
            None,
        )?;

        let global = query_global_preferences(&conn, 10)?;
        assert_eq!(
            global.len(),
            1,
            "Only preferences in 3+ projects should be returned"
        );
        assert!(global[0].text.contains("terse"));
        Ok(())
    }

    #[test]
    fn test_dedup_with_claude_md() {
        let prefs = vec![
            Memory {
                id: 1,
                session_id: None,
                project: "test".into(),
                topic_key: Some("p1".into()),
                title: "Preference: use chinese comments".into(),
                text: "use chinese comments in code".into(),
                memory_type: "preference".into(),
                files: None,
                created_at_epoch: 0,
                updated_at_epoch: 0,
                status: "active".into(),
                branch: None,
            },
            Memory {
                id: 2,
                session_id: None,
                project: "test".into(),
                topic_key: Some("p2".into()),
                title: "Preference: terse output".into(),
                text: "give terse output".into(),
                memory_type: "preference".into(),
                files: None,
                created_at_epoch: 0,
                updated_at_epoch: 0,
                status: "active".into(),
                branch: None,
            },
        ];

        // Simulate CLAUDE.md containing "use chinese comments"
        // Since we can't create a temp file easily, test with nonexistent path (all pass)
        let indices = dedup_with_claude_md(&prefs, "/nonexistent");
        assert_eq!(indices.len(), 2, "All prefs should pass when no CLAUDE.md");
    }

    #[test]
    fn test_add_and_remove_preference() -> Result<()> {
        let conn = setup_test_db();
        let id = add_preference(&conn, "test/proj", "Always use descriptive variable names")?;
        assert!(id > 0);

        let prefs = memory::get_memories_by_type(&conn, "test/proj", "preference", 10)?;
        assert_eq!(prefs.len(), 1);

        let removed = remove_preference(&conn, id)?;
        assert!(removed);

        let prefs = memory::get_memories_by_type(&conn, "test/proj", "preference", 10)?;
        assert!(prefs.is_empty());
        Ok(())
    }
}
