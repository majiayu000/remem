use super::{MarkdownMemoryDocument, MarkdownMemoryMetadata, EXPORT_VERSION, META_END, META_START};
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn render_markdown_memory(doc: &MarkdownMemoryDocument) -> String {
    let metadata = serde_json::to_string_pretty(&doc.metadata)
        .expect("markdown export metadata should serialize");
    format!(
        "{META_START}\n{metadata}\n{META_END}\n\n# {}\n\n{}",
        heading_title(&doc.metadata.title),
        doc.content
    )
}

pub(super) fn parse_markdown_memory(raw: &str) -> Result<MarkdownMemoryDocument> {
    let body = raw
        .strip_prefix(META_START)
        .ok_or_else(|| anyhow!("missing remem markdown metadata start marker"))?;
    let end_marker = format!("\n{META_END}");
    let end = body
        .find(&end_marker)
        .ok_or_else(|| anyhow!("missing remem markdown metadata end marker"))?;
    let mut metadata: MarkdownMemoryMetadata =
        serde_json::from_str(body[..end].trim()).context("parse remem markdown metadata")?;
    let content_start = end + end_marker.len();
    let (heading_title, content) = strip_visible_heading(&body[content_start..]);
    if let Some(title) = heading_title {
        metadata.title = title;
    }
    Ok(MarkdownMemoryDocument { metadata, content })
}

pub(super) fn validate_markdown_metadata(doc: &MarkdownMemoryDocument) -> Result<()> {
    if doc.metadata.remem_export_version != EXPORT_VERSION {
        anyhow::bail!(
            "unsupported remem markdown export version {}",
            doc.metadata.remem_export_version
        );
    }
    if crate::memory::MemoryType::parse(&doc.metadata.memory_type).is_none() {
        anyhow::bail!("unsupported memory_type {}", doc.metadata.memory_type);
    }
    if doc.metadata.project.trim().is_empty() {
        anyhow::bail!("markdown memory project must not be empty");
    }
    if doc.metadata.title.trim().is_empty() {
        anyhow::bail!("markdown memory title must not be empty");
    }
    if doc.content.trim().is_empty() {
        anyhow::bail!("markdown memory content must not be empty");
    }
    if !matches!(
        doc.metadata.status.as_str(),
        "active" | "stale" | "archived"
    ) {
        anyhow::bail!("unsupported markdown memory status {}", doc.metadata.status);
    }
    if !matches!(doc.metadata.scope.as_str(), "project" | "global") {
        anyhow::bail!("unsupported markdown memory scope {}", doc.metadata.scope);
    }
    let reference_time = doc
        .metadata
        .reference_time_epoch
        .or(Some(doc.metadata.created_at_epoch));
    validate_reference_time(
        doc.metadata.source_id,
        &doc.metadata.title,
        &doc.content,
        reference_time,
    )
}

pub(super) fn markdown_file_name(doc: &MarkdownMemoryDocument) -> String {
    let source_id = doc.metadata.source_id.unwrap_or_default();
    let label = doc
        .metadata
        .topic_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&doc.metadata.title);
    format!(
        "{source_id:06}-{}-{}.md",
        slug_component(&doc.metadata.memory_type),
        slug_component(label)
    )
}

pub(super) fn markdown_files(source: &Path) -> Result<Vec<PathBuf>> {
    if source.is_file() {
        let is_markdown = source
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
        return if is_markdown {
            Ok(vec![source.to_path_buf()])
        } else {
            Ok(Vec::new())
        };
    }
    if !source.exists() {
        anyhow::bail!("markdown source not found at {}", source.display());
    }
    let mut files = Vec::new();
    collect_markdown_files(source, &mut files)?;
    files.sort();
    Ok(files)
}

pub(super) fn normalized_topic_key(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn synthesized_markdown_topic_key(path: &Path, title: &str) -> String {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(title);
    format!("markdown-{}", slug_component(stem))
}

fn strip_visible_heading(raw: &str) -> (Option<String>, String) {
    let content = trim_heading_breaks(raw);
    let Some(after_hash) = content.strip_prefix("# ") else {
        return (None, content.to_string());
    };
    let Some(line_end) = after_hash.find('\n') else {
        let heading = after_hash.trim_end_matches('\r');
        return (Some(unescape_heading_title(heading)), String::new());
    };
    let heading = after_hash[..line_end].trim_end_matches('\r');
    let content = trim_heading_breaks(&after_hash[line_end + 1..]);
    (Some(unescape_heading_title(heading)), content.to_string())
}

fn trim_heading_breaks(value: &str) -> &str {
    value.trim_start_matches(['\r', '\n'])
}

fn heading_title(title: &str) -> String {
    title
        .lines()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('#', "\\#")
}

fn unescape_heading_title(title: &str) -> String {
    title.replace("\\#", "#")
}

fn slug_component(value: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "memory".to_string()
    } else {
        slug.chars().take(80).collect()
    }
}

fn collect_markdown_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_markdown_files(&path, files)?;
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            files.push(path);
        }
    }
    Ok(())
}

fn validate_reference_time(
    source_id: Option<i64>,
    title: &str,
    content: &str,
    reference_time_epoch: Option<i64>,
) -> Result<()> {
    let has_relative_time = crate::memory::reference_time::contains_relative_time_reference(title)
        || crate::memory::reference_time::contains_relative_time_reference(content);
    if has_relative_time && reference_time_epoch.is_none_or(|epoch| epoch <= 0) {
        let source = source_id
            .map(|id| format!("source memory id={id}"))
            .unwrap_or_else(|| "markdown memory".to_string());
        anyhow::bail!("{source}: relative dates require a positive reference_time_epoch");
    }
    Ok(())
}
