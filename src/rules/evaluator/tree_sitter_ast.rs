const DYNAMIC_SHELL_WORD: &str = "__remem_dynamic_shell_word__";

pub(super) fn command_segments(source: &str) -> Result<Option<Vec<Vec<String>>>, String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_bash::LANGUAGE.into())
        .map_err(|error| format!("could not load Bash tree-sitter grammar: {error}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "Bash tree-sitter parser returned no syntax tree".to_string())?;
    if tree.root_node().has_error() {
        return Ok(None);
    }

    let mut segments = Vec::new();
    let mut pending = vec![tree.root_node()];
    while let Some(node) = pending.pop() {
        if node.kind() == "heredoc_body" {
            continue;
        }
        if node.kind() == "command" {
            let Some(tokens) = command_tokens(node, source)? else {
                return Ok(None);
            };
            if !tokens.is_empty() {
                segments.push(tokens);
            }
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    Ok(Some(segments))
}

fn command_tokens(
    node: tree_sitter::Node<'_>,
    source: &str,
) -> Result<Option<Vec<String>>, String> {
    let name = node
        .child_by_field_name("name")
        .ok_or_else(|| "Bash command node is missing its name".to_string())?;
    if matches!(node_text(name, source)?, "{" | "}") {
        return Ok(None);
    }

    let mut tokens = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "variable_assignment" && child.end_byte() <= name.start_byte() {
            tokens.push(node_text(child, source)?.to_string());
        }
    }
    tokens.push(command_word(name, source)?);
    let mut cursor = node.walk();
    for argument in node.children_by_field_name("argument", &mut cursor) {
        tokens.push(command_word(argument, source)?);
    }
    Ok(Some(tokens))
}

fn command_word(node: tree_sitter::Node<'_>, source: &str) -> Result<String, String> {
    Ok(static_word(node, source)?.unwrap_or_else(|| DYNAMIC_SHELL_WORD.to_string()))
}

fn static_word(node: tree_sitter::Node<'_>, source: &str) -> Result<Option<String>, String> {
    match node.kind() {
        "command_name" => {
            let mut cursor = node.walk();
            let Some(child) = node.named_children(&mut cursor).next() else {
                return Ok(Some(node_text(node, source)?.to_string()));
            };
            static_word(child, source)
        }
        "word" | "number" | "string_content" => {
            decode_unquoted_word(node_text(node, source)?).map(Some)
        }
        "raw_string" => {
            let text = node_text(node, source)?;
            Ok(text
                .strip_prefix('\'')
                .and_then(|text| text.strip_suffix('\''))
                .map(str::to_string))
        }
        "string" => static_double_quoted_word(node, source),
        "concatenation" => {
            let mut value = String::new();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                let Some(part) = static_word(child, source)? else {
                    return Ok(None);
                };
                value.push_str(&part);
            }
            Ok(Some(value))
        }
        _ => Ok(None),
    }
}

fn static_double_quoted_word(
    node: tree_sitter::Node<'_>,
    source: &str,
) -> Result<Option<String>, String> {
    let mut cursor = node.walk();
    if node
        .named_children(&mut cursor)
        .any(|child| child.kind() != "string_content")
    {
        return Ok(None);
    }
    let text = node_text(node, source)?;
    let Some(text) = text
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
    else {
        return Ok(None);
    };
    decode_double_quoted_word(text).map(Some)
}

fn decode_unquoted_word(text: &str) -> Result<String, String> {
    let mut value = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            value.push(ch);
            continue;
        }
        let escaped = chars
            .next()
            .ok_or_else(|| "Bash word ends with an incomplete escape".to_string())?;
        if escaped != '\n' {
            value.push(escaped);
        }
    }
    Ok(value)
}

fn decode_double_quoted_word(text: &str) -> Result<String, String> {
    let mut value = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            value.push(ch);
            continue;
        }
        let escaped = chars
            .next()
            .ok_or_else(|| "Bash string ends with an incomplete escape".to_string())?;
        if escaped == '\n' {
            continue;
        }
        if matches!(escaped, '$' | '`' | '"' | '\\') {
            value.push(escaped);
        } else {
            value.push('\\');
            value.push(escaped);
        }
    }
    Ok(value)
}

fn node_text<'a>(node: tree_sitter::Node<'_>, source: &'a str) -> Result<&'a str, String> {
    source
        .get(node.byte_range())
        .ok_or_else(|| "Bash syntax node is outside the command text".to_string())
}
