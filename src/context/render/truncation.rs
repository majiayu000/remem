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
    let mut inside_memory_item = false;

    for line in body.split_inclusive('\n') {
        let line_start_byte = byte_pos;
        let line_start_chars = chars_seen;
        let line_chars = char_len(line);
        let line_end_byte = line_start_byte + line.len();
        let line_end_chars = line_start_chars + line_chars;
        let starts_memory_item = line.starts_with("**#");
        let blank_line = line.trim().is_empty();

        if line_start_chars <= keep_chars
            && (!inside_memory_item || starts_memory_item || blank_line)
        {
            last_boundary = line_start_byte;
        }
        if line_start_chars >= keep_chars || line_end_chars > keep_chars {
            break;
        }

        if starts_memory_item {
            inside_memory_item = true;
        } else if inside_memory_item {
            if blank_line {
                inside_memory_item = false;
                last_boundary = line_end_byte;
            }
        } else {
            last_boundary = line_end_byte;
        }

        byte_pos = line_end_byte;
        chars_seen = line_end_chars;
    }

    body[..last_boundary].to_string()
}
