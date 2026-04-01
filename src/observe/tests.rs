use super::parse::parse_native_memory_frontmatter;
use super::path::extract_project_from_memory_path;

#[test]
fn parse_frontmatter_full() {
    let content =
        "---\nname: my memory\ndescription: test\ntype: feedback\n---\nBody content here.";
    let (title, memory_type, body) = parse_native_memory_frontmatter(content);
    assert_eq!(title, "my memory");
    assert_eq!(memory_type, "preference");
    assert_eq!(body.trim(), "Body content here.");
}

#[test]
fn parse_frontmatter_missing() {
    let content = "Just plain text, no frontmatter.";
    let (title, memory_type, body) = parse_native_memory_frontmatter(content);
    assert_eq!(title, "Untitled memory");
    assert_eq!(memory_type, "discovery");
    assert_eq!(body, content);
}

#[test]
fn parse_frontmatter_project_type() {
    let content = "---\nname: deploy notes\ntype: project\n---\nContent.";
    let (_, memory_type, _) = parse_native_memory_frontmatter(content);
    assert_eq!(memory_type, "discovery");
}

#[test]
fn extract_project_from_path() {
    let path = "/Users/lifcc/.claude/projects/-Users-lifcc-Desktop-code-AI-tools-remem/memory/feedback_quality.md";
    let project = extract_project_from_memory_path(path);
    assert_eq!(project, "/Users/lifcc/Desktop/code/AI/tools/remem");
}

#[test]
fn extract_project_short_slug() {
    let path = "/Users/x/.claude/projects/-myproject/memory/foo.md";
    let project = extract_project_from_memory_path(path);
    assert_eq!(project, "/myproject");
}
