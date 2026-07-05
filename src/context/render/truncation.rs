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
    let mut last_boundary = 0usize;
    let mut open_item = None;

    for line in body.split_inclusive('\n') {
        let line_start_byte = byte_pos;
        let line_start_chars = chars_seen;
        let line_chars = char_len(line);
        let line_end_byte = line_start_byte + line.len();
        let line_end_chars = line_start_chars + line_chars;
        let starts_memory_item = looks_like_memory_item_header(line);
        let starts_structured_list_item = looks_like_structured_list_item(line);
        let starts_list_item = line.starts_with("- ");
        let starts_section = line.starts_with("## ");
        let starts_item_boundary = match open_item {
            Some(OpenItem::Memory) => starts_memory_item,
            Some(OpenItem::List) => starts_structured_list_item,
            None => starts_memory_item || starts_list_item,
        };

        if line_start_chars <= keep_chars && (starts_section || starts_item_boundary) {
            last_boundary = line_start_byte;
        }
        if line_start_chars >= keep_chars {
            break;
        }
        if line_end_chars > keep_chars {
            if open_item.is_none() {
                if let Some(boundary) = index_line_item_boundary_before_cut(
                    line,
                    line_start_byte,
                    line_start_chars,
                    keep_chars,
                ) {
                    last_boundary = boundary;
                }
            }
            break;
        }

        if starts_section {
            open_item = None;
            last_boundary = line_end_byte;
        } else if starts_memory_item {
            open_item = Some(OpenItem::Memory);
        } else if starts_structured_list_item || (open_item.is_none() && starts_list_item) {
            open_item = Some(OpenItem::List);
        } else if open_item.is_none() {
            last_boundary = line_end_byte;
        }

        byte_pos = line_end_byte;
        chars_seen = line_end_chars;
    }

    body[..last_boundary].to_string()
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum OpenItem {
    Memory,
    List,
}

fn looks_like_memory_item_header(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("**#") else {
        return false;
    };
    let id_digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    id_digits > 0 && rest[id_digits..].starts_with(' ') && line.contains("** (")
}

fn looks_like_structured_list_item(line: &str) -> bool {
    line.starts_with("- **") || line.starts_with("- #")
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
