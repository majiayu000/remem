use super::constants::BASH_SKIP_PREFIXES;

pub fn should_skip_bash_command(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    let lowered = trimmed.to_lowercase();

    BASH_SKIP_PREFIXES
        .iter()
        .any(|prefix| lowered.starts_with(prefix))
        || lowered.contains("| grep ")
        || is_read_only_polling_cmd(&lowered)
}

fn is_read_only_polling_cmd(cmd_lower: &str) -> bool {
    let is_curl = cmd_lower.starts_with("curl ");
    let has_mutation_method = cmd_lower.contains("-x post")
        || cmd_lower.contains("-x put")
        || cmd_lower.contains("-x patch")
        || cmd_lower.contains("-x delete")
        || cmd_lower.contains("--request post")
        || cmd_lower.contains("--request put")
        || cmd_lower.contains("--request patch")
        || cmd_lower.contains("--request delete");

    if is_curl && !has_mutation_method {
        return true;
    }

    cmd_lower.starts_with("sleep ") && cmd_lower.contains("&& curl ")
}
