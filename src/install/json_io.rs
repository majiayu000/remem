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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(value)?;
    std::fs::write(path, content).with_context(|| format!("写入 {} 失败", path.display()))
}
