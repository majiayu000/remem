use super::super::format::char_len;

pub(super) fn truncate_context_body_at_stable_boundary(body: &str, keep_chars: usize) -> String {
    if keep_chars == 0 {
        return String::new();
    }
    if char_len(body) <= keep_chars {
        return body.to_string();
    }

    let mut byte_pos = 0usize;
    let mut chars_seen = 0usize;
    let mut lines = Vec::new();
    for line in body.split_inclusive('\n') {
        let line_chars = char_len(line);
        let line_start_byte = byte_pos;
        let line_start_chars = chars_seen;
        byte_pos += line.len();
        chars_seen += line_chars;
        lines.push(LineInfo {
            text: line,
            start_byte: line_start_byte,
            start_chars: line_start_chars,
            end_byte: byte_pos,
            end_chars: chars_seen,
        });
    }

    let mut last_boundary = 0usize;
    let mut current_section: Option<Section> = None;
    let mut open_item = None;

    for (index, line) in lines.iter().enumerate() {
        let previous_line = index
            .checked_sub(1)
            .and_then(|previous| lines.get(previous))
            .map(|previous| previous.text);
        let next_line = lines.get(index + 1).map(|next| next.text);
        let next_next_line = lines.get(index + 2).map(|next| next.text);
        let next_next_next_line = lines.get(index + 3).map(|next| next.text);
        let section = known_section(line.text);
        let starts_section = starts_rendered_section(
            line.text,
            current_section,
            open_item,
            previous_line,
            next_line,
        );
        let starts_item_boundary = starts_item_boundary(line.text, current_section);
        let completes_item = item_completion_separator(
            next_line,
            next_next_line,
            next_next_next_line,
            current_section,
            open_item,
        );
        let completes_index_line = current_section == Some(Section::Index)
            && complete_index_line_boundary(line.text).is_some();
        let starts_partial_memory_item =
            starts_partial_memory_item(line.text, current_section, open_item);

        if line.start_chars <= keep_chars && (starts_section || starts_item_boundary) {
            last_boundary = line.start_byte;
        }
        if line.start_chars >= keep_chars {
            break;
        }
        if line.end_chars > keep_chars {
            if open_item.is_none() && current_section == Some(Section::Index) {
                if let Some(boundary) = index_line_item_boundary_before_cut(
                    line.text,
                    line.start_byte,
                    line.start_chars,
                    keep_chars,
                ) {
                    last_boundary = boundary;
                }
            }
            break;
        }

        if starts_section {
            current_section = section;
            open_item = None;
            last_boundary = line.end_byte;
        } else if starts_item_boundary {
            open_item = current_section.and_then(Section::item_kind);
        } else if starts_partial_memory_item {
            open_item = Some(OpenItem::Memory);
        } else if completes_item
            || completes_index_line
            || (open_item.is_none() && current_section.is_none())
        {
            if completes_item {
                open_item = None;
            }
            last_boundary = line.end_byte;
        }
    }

    body[..last_boundary].to_string()
}

struct LineInfo<'a> {
    text: &'a str,
    start_byte: usize,
    start_chars: usize,
    end_byte: usize,
    end_chars: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum OpenItem {
    Memory,
    List,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Section {
    ContextLoadErrors,
    Preferences,
    Lessons,
    Core,
    Index,
    Workstreams,
    Sessions,
    DebugTrace,
}

impl Section {
    fn order(self) -> u8 {
        match self {
            Section::ContextLoadErrors => 0,
            Section::Preferences => 1,
            Section::Lessons => 2,
            Section::Core => 3,
            Section::Index => 4,
            Section::Workstreams => 5,
            Section::Sessions => 6,
            Section::DebugTrace => 7,
        }
    }

    fn item_kind(self) -> Option<OpenItem> {
        match self {
            Section::Lessons | Section::Core => Some(OpenItem::Memory),
            Section::ContextLoadErrors
            | Section::Preferences
            | Section::Workstreams
            | Section::Sessions => Some(OpenItem::List),
            Section::Index | Section::DebugTrace => None,
        }
    }
}

fn known_section(line: &str) -> Option<Section> {
    match line.trim_end_matches('\n') {
        "## Context Load Errors" => Some(Section::ContextLoadErrors),
        "## Your Preferences (always apply these)" => Some(Section::Preferences),
        "## Lessons" => Some(Section::Lessons),
        "## Core" => Some(Section::Core),
        "## Index" => Some(Section::Index),
        "## WorkStreams" => Some(Section::Workstreams),
        "## Sessions" => Some(Section::Sessions),
        "## Debug Trace" => Some(Section::DebugTrace),
        _ => None,
    }
}

fn starts_item_boundary(line: &str, section: Option<Section>) -> bool {
    match section.and_then(Section::item_kind) {
        Some(OpenItem::Memory) => looks_like_memory_item_header(line),
        Some(OpenItem::List) => match section {
            Some(Section::Workstreams) => line.starts_with("- #"),
            Some(Section::Sessions) => line.starts_with("- **"),
            Some(Section::ContextLoadErrors) => line.starts_with("- "),
            Some(Section::Preferences) => line.starts_with("- "),
            _ => false,
        },
        None => false,
    }
}

fn starts_partial_memory_item(
    line: &str,
    section: Option<Section>,
    open_item: Option<OpenItem>,
) -> bool {
    open_item.is_none()
        && section.and_then(Section::item_kind) == Some(OpenItem::Memory)
        && line.starts_with("**#")
}

fn item_completion_separator(
    next_line: Option<&str>,
    next_next_line: Option<&str>,
    next_next_next_line: Option<&str>,
    section: Option<Section>,
    open_item: Option<OpenItem>,
) -> bool {
    if open_item.is_none() {
        return false;
    }
    let Some(next_line) = next_line else {
        return true;
    };
    if next_line.trim().is_empty() {
        return next_next_line.is_some_and(|line| {
            structural_boundary_after_blank(line, section, next_next_next_line)
        });
    }
    structural_item_boundary(next_line, section)
}

fn starts_rendered_section(
    line: &str,
    current_section: Option<Section>,
    open_item: Option<OpenItem>,
    previous_line: Option<&str>,
    following_line: Option<&str>,
) -> bool {
    let Some(next_section) = known_section(line) else {
        return false;
    };
    if current_section.is_some_and(|current| next_section.order() <= current.order()) {
        return false;
    }
    open_item.is_none()
        || (previous_line.is_some_and(|line| line.trim().is_empty())
            && section_body_can_start(next_section, following_line))
}

fn structural_boundary_after_blank(
    line: &str,
    section: Option<Section>,
    following_line: Option<&str>,
) -> bool {
    if known_section(line).is_some_and(|next| {
        section.is_none_or(|current| next.order() > current.order())
            && section_body_can_start(next, following_line)
    }) {
        return true;
    }
    structural_item_boundary(line, section)
}

fn structural_item_boundary(line: &str, section: Option<Section>) -> bool {
    starts_item_boundary(line, section) || starts_partial_memory_item(line, section, None)
}

fn section_body_can_start(section: Section, line: Option<&str>) -> bool {
    let Some(line) = line else {
        return false;
    };
    match section {
        Section::ContextLoadErrors | Section::Preferences | Section::DebugTrace => {
            line.starts_with("- ")
        }
        Section::Lessons | Section::Core => {
            looks_like_memory_item_header(line)
                || starts_partial_memory_item(line, Some(section), None)
        }
        Section::Index => looks_like_index_line_start(line),
        Section::Workstreams => line.starts_with("- #"),
        Section::Sessions => line.starts_with("- **"),
    }
}

fn looks_like_memory_item_header(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("**#") else {
        return false;
    };
    let id_digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    id_digits > 0 && rest[id_digits..].starts_with(' ') && line.contains("** (")
}

fn index_line_item_boundary_before_cut(
    line: &str,
    line_start_byte: usize,
    line_start_chars: usize,
    keep_chars: usize,
) -> Option<usize> {
    if !looks_like_index_line_start(line) {
        return None;
    }
    let chars_before_cut = keep_chars.checked_sub(line_start_chars)?;
    let mut entry_start = line.find(": #")? + 2;
    let mut boundary = None;
    while entry_start < line.len() {
        let Some(entry_end) = complete_index_entry_end(&line[entry_start..]) else {
            break;
        };
        let entry_end = entry_start + entry_end;
        if char_len(&line[..entry_end]) > chars_before_cut {
            break;
        }
        boundary = Some(line_start_byte + entry_end);
        if line[entry_end..].starts_with(" | #") {
            entry_start = entry_end + " | ".len();
        } else {
            break;
        }
    }
    boundary
}

fn complete_index_entry_end(entry_and_tail: &str) -> Option<usize> {
    for (close_byte, _) in entry_and_tail.match_indices(')') {
        let entry_end = close_byte + 1;
        if !looks_like_complete_index_entry(&entry_and_tail[..entry_end]) {
            continue;
        }
        let after_entry = &entry_and_tail[entry_end..];
        if after_entry.starts_with(" | #") || after_entry.trim().is_empty() {
            return Some(entry_end);
        }
    }
    None
}

fn complete_index_line_boundary(line: &str) -> Option<usize> {
    if !looks_like_index_line_start(line) {
        return None;
    }
    let mut entry_start = line.find(": #")? + 2;
    let mut boundary = None;
    while entry_start < line.len() {
        let entry_end = complete_index_entry_end(&line[entry_start..])?;
        let entry_end = entry_start + entry_end;
        boundary = Some(entry_end);
        if line[entry_end..].starts_with(" | #") {
            entry_start = entry_end + " | ".len();
            continue;
        }
        if line[entry_end..].trim().is_empty() {
            return boundary;
        }
        return None;
    }
    boundary
}

fn looks_like_index_line_start(line: &str) -> bool {
    line.starts_with("**") && !line.starts_with("**#") && line.contains(": #")
}

fn looks_like_complete_index_entry(entry: &str) -> bool {
    let entry = entry.trim();
    let Some(id_and_tail) = entry.strip_prefix('#') else {
        return false;
    };
    let id_digits = id_and_tail.bytes().take_while(u8::is_ascii_digit).count();
    if id_digits == 0 || !id_and_tail[id_digits..].starts_with(' ') {
        return false;
    }
    let Some(metadata_start) = entry.rfind(" (") else {
        return false;
    };
    entry.ends_with(')') && entry[metadata_start..].contains("; ")
}
