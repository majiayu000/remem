//! Render v2 schema status lines for `remem status` (and later `remem doctor`).
//! Pure formatting, no DB connection required — only checks for the presence
//! of the v2 file at `crate::v2_db::default_v2_db_path()`.

use std::path::Path;

use crate::v2_db::default_v2_db_path;

/// Render the v2 status block at the default path (`~/.remem/v2.sqlite`).
pub fn format_v2_summary() -> Vec<String> {
    format_v2_summary_at(&default_v2_db_path())
}

/// Render the v2 status block for an arbitrary path (testable entry).
pub fn format_v2_summary_at(path: &Path) -> Vec<String> {
    let mut lines = vec!["v2 schema:".to_string()];
    if path.exists() {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        lines.push(format!("  Location: {}", path.display()));
        lines.push("  Status:   initialized".to_string());
        lines.push(format!("  Size:     {:.1} MB", size as f64 / 1_048_576.0));
    } else {
        lines.push(format!(
            "  Location: {} (would be created here)",
            path.display()
        ));
        lines.push("  Status:   not initialized".to_string());
        lines.push(
            "  Hint:     run `remem admin reset-v2 --confirm-destructive` to create"
                .to_string(),
        );
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_marker_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "remem-v2-status-{label}-{}-{}.tmp",
            std::process::id(),
            nonce
        ))
    }

    #[test]
    fn summary_indicates_initialized_when_path_exists() {
        let path = unique_marker_path("exists");
        std::fs::write(&path, b"placeholder").unwrap();
        let lines = format_v2_summary_at(&path);
        let body = lines.join("\n");
        assert!(body.contains("v2 schema:"), "got: {body}");
        assert!(body.contains("Status:   initialized"), "got: {body}");
        assert!(body.contains("Size:"), "got: {body}");
        assert!(!body.contains("not initialized"), "got: {body}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn summary_indicates_not_initialized_when_path_missing() {
        let path = unique_marker_path("missing");
        // Ensure absent (best-effort cleanup; nonce already makes it unique).
        let _ = std::fs::remove_file(&path);
        let lines = format_v2_summary_at(&path);
        let body = lines.join("\n");
        assert!(body.contains("not initialized"), "got: {body}");
        assert!(body.contains("remem admin reset-v2"), "got: {body}");
    }

    #[test]
    fn summary_first_line_is_section_header() {
        let path = unique_marker_path("hdr");
        let lines = format_v2_summary_at(&path);
        assert_eq!(lines[0], "v2 schema:");
    }
}
