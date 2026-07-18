use brush_parser::ParserOptions;

use super::function_args::{
    bare_shell_positional_fields, expand_shell_command, has_shell_positional_reference,
};
use super::static_words::static_word_pieces;
use super::DYNAMIC_SHELL_WORD;

pub(super) fn shell_positional_variant_fields(
    source: &str,
    options: &ParserOptions,
    zero_argument: Option<&str>,
    arguments: &[String],
    possible_arguments: &[Vec<String>],
) -> Result<Option<Vec<String>>, String> {
    if !has_shell_positional_reference(source) {
        return Ok(None);
    }
    let mut combined = Vec::new();
    for arguments in std::iter::once(arguments).chain(possible_arguments.iter().map(Vec::as_slice))
    {
        if let Some(fields) =
            bare_shell_positional_fields(source, options, zero_argument, arguments)?
        {
            combined.extend(fields);
        } else if possible_arguments.is_empty() {
            return Ok(None);
        } else {
            let expanded = expand_shell_command(source, options, zero_argument, arguments)?;
            let pieces = brush_parser::word::parse(&expanded, options)
                .map_err(|error| format!("Bash positional variant parse error: {error}"))?;
            combined.push(
                static_word_pieces(&pieces).unwrap_or_else(|| DYNAMIC_SHELL_WORD.to_string()),
            );
        }
    }
    Ok(Some(combined))
}
