use super::super::render::{enforce_total_char_limit, enforce_total_char_limit_preserving_footer};

#[test]
fn enforce_total_char_limit_truncates_rendered_output() {
    let mut output = format!("{}{}", "# [/tmp/demo] context\n", "x".repeat(500));

    enforce_total_char_limit(&mut output, 120);

    assert!(output.chars().count() <= 120);
    assert!(output.contains("REMEM_CONTEXT_TOTAL_CHAR_LIMIT"));
}

#[test]
fn enforce_total_char_limit_drops_partial_single_line_item() {
    let marker = "\n[remem context truncated to REMEM_CONTEXT_TOTAL_CHAR_LIMIT]\n";
    let tail = "second session tail should not survive as a partial item. ".repeat(8);
    let mut output = String::from("# [/tmp/demo] context\n\n## Sessions\n");
    output.push_str("- **2026-07-05** 12:00 complete session remains\n");
    output.push_str(&format!("- **2026-07-06** 12:01 {tail}\n"));
    let tail_start = output
        .find("second session tail")
        .expect("tail session should be present");
    let keep_chars = output[..tail_start].chars().count() + "second session".chars().count();
    let char_limit = keep_chars + marker.chars().count();

    assert!(char_limit < output.chars().count());

    enforce_total_char_limit(&mut output, char_limit);

    assert!(output.chars().count() <= char_limit);
    assert!(output.contains("complete session remains"));
    assert!(!output.contains("2026-07-06"));
    assert!(!output.contains("second session tail"));
    assert!(output.contains("REMEM_CONTEXT_TOTAL_CHAR_LIMIT"));
}

#[test]
fn enforce_total_char_limit_drops_partial_multiline_memory_item() {
    let marker = "\n[remem context truncated to REMEM_CONTEXT_TOTAL_CHAR_LIMIT]\n";
    let tail = "second paragraph should not survive as a partial memory item. ".repeat(8);
    let mut output = String::from("# [/tmp/demo] context\n\n## Core\n");
    output.push_str("**#1 First complete** (decision, 2026-07-05; local, trusted)\n");
    output.push_str("first memory remains intact\n");
    output.push_str("**#2 Multiline tail** (decision, 2026-07-05; local, trusted)\n");
    output.push_str("first paragraph of the second memory\n\n");
    output.push_str(&tail);
    output.push('\n');
    let tail_start = output
        .find("second paragraph")
        .expect("tail memory paragraph should be present");
    let keep_chars = output[..tail_start].chars().count() + "second paragraph".chars().count();
    let char_limit = keep_chars + marker.chars().count();

    assert!(char_limit < output.chars().count());

    enforce_total_char_limit(&mut output, char_limit);

    assert!(output.chars().count() <= char_limit);
    assert!(output.contains("First complete"));
    assert!(output.contains("first memory remains intact"));
    assert!(!output.contains("Multiline tail"));
    assert!(!output.contains("first paragraph of the second memory"));
    assert!(!output.contains("second paragraph"));
    assert!(output.contains("REMEM_CONTEXT_TOTAL_CHAR_LIMIT"));
}

#[test]
fn enforce_total_char_limit_drops_partial_multiline_list_item() {
    let marker = "\n[remem context truncated to REMEM_CONTEXT_TOTAL_CHAR_LIMIT]\n";
    let tail = "list continuation should not survive as partial context. ".repeat(8);
    let mut output = String::from("# [/tmp/demo] context\n\n## Preferences\n");
    output.push_str("- First complete preference remains\n");
    output.push_str("- Second multiline preference starts here\n");
    output.push_str("continued preference detail\n\n");
    output.push_str(&tail);
    output.push('\n');
    let tail_start = output
        .find("list continuation")
        .expect("tail list continuation should be present");
    let keep_chars = output[..tail_start].chars().count() + "list continuation".chars().count();
    let char_limit = keep_chars + marker.chars().count();

    assert!(char_limit < output.chars().count());

    enforce_total_char_limit(&mut output, char_limit);

    assert!(output.chars().count() <= char_limit);
    assert!(output.contains("First complete preference remains"));
    assert!(!output.contains("Second multiline preference"));
    assert!(!output.contains("continued preference detail"));
    assert!(!output.contains("list continuation"));
    assert!(output.contains("REMEM_CONTEXT_TOTAL_CHAR_LIMIT"));
}

#[test]
fn enforce_total_char_limit_preserves_complete_index_entries_before_cut() {
    let marker = "\n[remem context truncated to REMEM_CONTEXT_TOTAL_CHAR_LIMIT]\n";
    let third = "Third index entry should not survive as a partial item. ".repeat(6);
    let mut output = String::from("# [/tmp/demo] context\n\n## Index\n");
    output.push_str("**decision** (3): #1 First stable item (2026-07-05; local, trusted)");
    output.push_str(" | #2 Second stable item (2026-07-05; local, trusted)");
    output.push_str(&format!(" | #3 {third}\n"));
    let third_start = output
        .find("#3 Third index entry")
        .expect("third index item should be present");
    let keep_chars = output[..third_start].chars().count() + "#3 Third index".chars().count();
    let char_limit = keep_chars + marker.chars().count();

    assert!(char_limit < output.chars().count());

    enforce_total_char_limit(&mut output, char_limit);

    assert!(output.chars().count() <= char_limit);
    assert!(output.contains("#1 First stable item"));
    assert!(output.contains("#2 Second stable item"));
    assert!(!output.contains("#3 Third index entry"));
    assert!(output.contains("REMEM_CONTEXT_TOTAL_CHAR_LIMIT"));
}

#[test]
fn enforce_total_char_limit_ignores_pipe_inside_index_title() {
    let marker = "\n[remem context truncated to REMEM_CONTEXT_TOTAL_CHAR_LIMIT]\n";
    let mut output = String::from("# [/tmp/demo] context\n\n## Index\n");
    let title_tail = "pipe text continues inside the same title. ".repeat(6);
    output.push_str(&format!(
        "**decision** (1): #1 First title with | {title_tail}(2026-07-05; local, trusted)\n"
    ));
    let pipe_start = output
        .find("| pipe text")
        .expect("title pipe should be present");
    let keep_chars = output[..pipe_start].chars().count() + "| pipe text".chars().count();
    let char_limit = keep_chars + marker.chars().count();

    assert!(char_limit < output.chars().count());

    enforce_total_char_limit(&mut output, char_limit);

    assert!(output.chars().count() <= char_limit);
    assert!(!output.contains("#1 First title"));
    assert!(!output.contains("| pipe"));
    assert!(output.contains("REMEM_CONTEXT_TOTAL_CHAR_LIMIT"));
}

#[test]
fn enforce_total_char_limit_preserves_index_entry_with_pipe_in_title() {
    let marker = "\n[remem context truncated to REMEM_CONTEXT_TOTAL_CHAR_LIMIT]\n";
    let tail = "Third index entry should not survive as a partial item. ".repeat(6);
    let mut output = String::from("# [/tmp/demo] context\n\n## Index\n");
    output
        .push_str("**decision** (3): #1 First title with | pipe text (2026-07-05; local, trusted)");
    output.push_str(" | #2 Second stable item (2026-07-05; local, trusted)");
    output.push_str(&format!(" | #3 {tail}\n"));
    let third_start = output
        .find("#3 Third index entry")
        .expect("third index item should be present");
    let keep_chars = output[..third_start].chars().count() + "#3 Third index".chars().count();
    let char_limit = keep_chars + marker.chars().count();

    assert!(char_limit < output.chars().count());

    enforce_total_char_limit(&mut output, char_limit);

    assert!(output.chars().count() <= char_limit);
    assert!(output.contains("#1 First title with | pipe text"));
    assert!(output.contains("#2 Second stable item"));
    assert!(!output.contains("#3 Third index entry"));
    assert!(output.contains("REMEM_CONTEXT_TOTAL_CHAR_LIMIT"));
}

#[test]
fn enforce_total_char_limit_preserves_footer_when_it_fits() {
    let footer = "22 context memories loaded. 2 core memories. 20 indexed memories. 5 preferences. 5 sessions.\n";
    let mut output = format!("{}{}{}", "# [/tmp/demo] context\n", "x".repeat(500), footer);

    enforce_total_char_limit_preserving_footer(&mut output, 180, footer);

    assert!(output.chars().count() <= 180);
    assert!(output.contains("REMEM_CONTEXT_TOTAL_CHAR_LIMIT"));
    assert!(output.ends_with(footer));
}

#[test]
fn enforce_total_char_limit_preserves_footer_and_drops_partial_memory_item() {
    let marker = "\n[remem context truncated to REMEM_CONTEXT_TOTAL_CHAR_LIMIT]\n";
    let footer = "\nLoaded\nBudget: 500 chars (~125 tokens) / 500, truncated: yes\n";
    let long_tail = "second memory tail should not survive as partial context. ".repeat(8);
    let mut body = String::from("remem context\nproject: /tmp/demo\nsource: compact\n\n## Core\n");
    body.push_str("**#1 First complete** (decision, 2026-07-05; local, trusted)\n");
    body.push_str("first memory remains intact\n");
    body.push_str(&format!(
        "**#2 Second tail** (decision, 2026-07-05; local, trusted)\n{long_tail}\n"
    ));
    let mut output = format!("{body}{footer}");
    let tail_start = body
        .find("second memory tail")
        .expect("tail memory should be present");
    let keep_chars = body[..tail_start].chars().count() + "second memory".chars().count();
    let char_limit = keep_chars + marker.chars().count() + footer.chars().count();

    assert!(char_limit < output.chars().count());

    enforce_total_char_limit_preserving_footer(&mut output, char_limit, footer);

    assert!(output.chars().count() <= char_limit);
    assert!(output.contains("First complete"));
    assert!(output.contains("first memory remains intact"));
    assert!(!output.contains("Second tail"));
    assert!(!output.contains("second memory tail"));
    assert!(output.contains("REMEM_CONTEXT_TOTAL_CHAR_LIMIT"));
    assert!(output.ends_with(footer));
}
