use anyhow::Result;
use std::path::Path;

const REMEM_FILE: &str = "remem_sessions.md";

pub(super) fn ensure_memory_index(memory_dir: &Path) -> Result<()> {
    let index_path = memory_dir.join("MEMORY.md");
    let pointer =
        format!("- [remem_sessions]({REMEM_FILE}) — 最近会话摘要和关键决策（remem 自动同步）");

    if index_path.exists() {
        let existing = std::fs::read_to_string(&index_path)?;
        if existing.contains(REMEM_FILE) {
            return Ok(());
        }
        let mut new_content = existing.trim_end().to_string();
        new_content.push_str("\n\n## Auto\n");
        new_content.push_str(&pointer);
        new_content.push('\n');
        std::fs::write(&index_path, new_content)?;
    } else {
        let content = format!("# Memory Index\n\n## Auto\n{}\n", pointer);
        std::fs::write(&index_path, content)?;
    }

    Ok(())
}
