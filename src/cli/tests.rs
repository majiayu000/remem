use super::cwd::resolve_cwd_arg;

#[test]
fn cli_resolve_cwd_arg_prefers_explicit_value() {
    assert_eq!(resolve_cwd_arg(Some("/tmp/remem".to_string())), "/tmp/remem");
}

#[test]
fn cli_resolve_cwd_arg_falls_back_to_current_dir() {
    let expected = std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    assert_eq!(resolve_cwd_arg(None), expected);
}
