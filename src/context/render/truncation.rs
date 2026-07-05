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
        let next_line = lines.get(index + 1).map(|next| next.text);
        let section = known_section(line.text);
        let starts_section = section.is_some_and(|next_section| {
            open_item.is_none()
                || current_section.is_some_and(|current| next_section.order() > current.order())
        });
        let starts_item_boundary = starts_item_boundary(line.text, current_section, open_item);
        let completes_item =
            item_completion_separator(line.text, next_line, current_section, open_item);

        if line.start_chars <= keep_chars
            && (starts_section || starts_item_boundary || completes_item)
        {
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
        } else if completes_item {
            open_item = None;
            last_boundary = line.end_byte;
        } else if open_item.is_none() {
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

fn starts_item_boundary(line: &str, section: Option<Section>, open_item: Option<OpenItem>) -> bool {
    match section.and_then(Section::item_kind) {
        Some(OpenItem::Memory) => looks_like_memory_item_header(line),
        Some(OpenItem::List) => match section {
            Some(Section::Workstreams) => line.starts_with("- #"),
            Some(Section::Sessions) => line.starts_with("- **"),
            Some(Section::ContextLoadErrors) => line.starts_with("- "),
            Some(Section::Preferences) => open_item.is_none() && line.starts_with("- "),
            _ => false,
        },
        None => false,
    }
}

fn item_completion_separator(
    line: &str,
    next_line: Option<&str>,
    section: Option<Section>,
    open_item: Option<OpenItem>,
) -> bool {
    if open_item.is_none() || !line.trim().is_empty() {
        return false;
    }
    let Some(next_line) = next_line else {
        return true;
    };
    let next_section = known_section(next_line);
    if next_section
        .is_some_and(|next| section.map_or(true, |current| next.order() > current.order()))
    {
        return true;
    }
    starts_item_boundary(next_line, section, open_item)
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
    if !line.starts_with("**") || line.starts_with("**#") || !line.contains(": #") {
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
