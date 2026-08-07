use anyhow::Result;

/// Long-running multi-connection services keep the multi-thread runtime.
/// Every other invocation — hook entrypoints (`context`, `session-init`,
/// `observe`, `summarize`) and one-shot CLI commands — runs on a
/// current_thread runtime so the hook floor path stops paying multi-thread
/// pool startup for purely blocking rusqlite work (GH-952).
const MULTI_THREAD_COMMANDS: &[&str] = &["mcp", "api", "worker", "eval-e2e", "bench", "dream"];

fn main() -> Result<()> {
    let subcommand = std::env::args().skip(1).find(|arg| !arg.starts_with('-'));
    let mut builder = if subcommand
        .as_deref()
        .is_some_and(|command| MULTI_THREAD_COMMANDS.contains(&command))
    {
        tokio::runtime::Builder::new_multi_thread()
    } else {
        tokio::runtime::Builder::new_current_thread()
    };
    builder.enable_all().build()?.block_on(remem::cli::run())
}
