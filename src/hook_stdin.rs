use anyhow::Result;
use std::io::IsTerminal;

/// Claude/Codex hook payloads share the Cursor stdin bound so no host can
/// stream an unbounded payload into hook memory.
const HOOK_STDIN_MAX_BYTES: usize = crate::cursor_hook::CURSOR_HOOK_STDIN_MAX_BYTES;

pub(crate) fn read_stdin_with_timeout(timeout_ms: u64) -> Result<Option<String>> {
    read_stdin_with_timeout_inner(timeout_ms, std::io::stdin().is_terminal())
}

fn read_stdin_with_timeout_inner(
    timeout_ms: u64,
    stdin_is_terminal: bool,
) -> Result<Option<String>> {
    use std::sync::mpsc;
    use std::time::Duration;

    if stdin_is_terminal {
        return Ok(None);
    }

    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let input = read_bounded_utf8(&mut std::io::stdin().lock(), HOOK_STDIN_MAX_BYTES);
        let _ = tx.send(input);
    });

    match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(Ok(input)) => {
            if input.trim().is_empty() {
                Ok(None)
            } else {
                Ok(Some(input))
            }
        }
        Ok(Err(error)) => Err(error),
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
        Err(mpsc::RecvTimeoutError::Disconnected) => Ok(None),
    }
}

fn read_bounded_utf8(reader: &mut dyn std::io::Read, max_bytes: usize) -> Result<String> {
    let bytes = crate::cursor_hook::input::read_bounded_hook_input(reader, max_bytes)?;
    String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("hook stdin is not valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_stdin_skips_timeout_read() -> Result<()> {
        let started = std::time::Instant::now();

        let input = read_stdin_with_timeout_inner(1000, true)?;

        assert!(input.is_none());
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
        Ok(())
    }

    #[test]
    fn bounded_read_accepts_payload_within_limit() -> Result<()> {
        let payload = b"{\"session_id\":\"s\"}".to_vec();

        let input = read_bounded_utf8(&mut payload.as_slice(), HOOK_STDIN_MAX_BYTES)?;

        assert_eq!(input, "{\"session_id\":\"s\"}");
        Ok(())
    }

    #[test]
    fn bounded_read_rejects_payload_over_limit() {
        let payload = vec![b'a'; HOOK_STDIN_MAX_BYTES + 1];

        let error = read_bounded_utf8(&mut payload.as_slice(), HOOK_STDIN_MAX_BYTES)
            .expect_err("oversized hook stdin must fail closed");

        assert!(error.to_string().contains("exceeds configured bound"));
    }

    #[test]
    fn bounded_read_rejects_invalid_utf8() {
        let payload = vec![0xff, 0xfe];

        let error = read_bounded_utf8(&mut payload.as_slice(), HOOK_STDIN_MAX_BYTES)
            .expect_err("non-UTF-8 hook stdin must fail closed");

        assert!(error.to_string().contains("not valid UTF-8"));
    }
}
