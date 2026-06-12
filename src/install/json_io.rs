use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;

pub(in crate::install) fn read_json_file(path: &PathBuf) -> Result<Value> {
    if path.exists() {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("读取 {} 失败", path.display()))?;
        serde_json::from_str(&content).with_context(|| format!("解析 {} 失败", path.display()))
    } else {
        Ok(serde_json::json!({}))
    }
}

pub(in crate::install) fn write_json_file(path: &PathBuf, value: &Value) -> Result<()> {
    let content = serde_json::to_string_pretty(value)?;
    crate::atomic_file::write_atomic(path, content)
        .with_context(|| format!("写入 {} 失败", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_json_failure_preserves_existing_file() -> Result<()> {
        let _guard = crate::atomic_file::failpoint_test_lock();
        let path = std::env::temp_dir().join(format!(
            "remem-json-atomic-{}-{}.json",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&path, r#"{"existing":true}"#)?;
        crate::atomic_file::fail_next_rename_for_test();

        let err = write_json_file(&path, &serde_json::json!({"existing": false}))
            .expect_err("injected failure must abort JSON write");
        assert!(format!("{err:?}").contains("injected atomic write failure"));
        assert_eq!(std::fs::read_to_string(&path)?, r#"{"existing":true}"#);
        crate::atomic_file::clear_failpoints_for_test();
        let _ = std::fs::remove_file(path);
        Ok(())
    }
}
