use anyhow::{anyhow, Result};
use std::io::{IsTerminal, Read};

pub(crate) const HOOK_STDIN_MAX_BYTES: usize = 1_048_576;

pub(crate) fn read_bounded_hook_input(reader: &mut dyn Read, max_bytes: usize) -> Result<Vec<u8>> {
    let mut limited = reader.take(max_bytes as u64 + 1);
    let mut buffer = Vec::new();
    limited
        .read_to_end(&mut buffer)
        .map_err(|error| anyhow!("hook stdin read failed: {error}"))?;
    if buffer.len() > max_bytes {
        return Err(anyhow!(
            "hook stdin exceeds configured bound (limit={max_bytes} bytes)"
        ));
    }
    Ok(buffer)
}

pub(crate) fn read_bounded_stdin_to_string() -> Result<String> {
    let bytes = read_bounded_hook_input(&mut std::io::stdin().lock(), HOOK_STDIN_MAX_BYTES)?;
    String::from_utf8(bytes).map_err(|error| anyhow!("hook stdin is not valid UTF-8: {error}"))
}

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
        let input = read_bounded_hook_input(&mut std::io::stdin(), HOOK_STDIN_MAX_BYTES)
            .and_then(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|error| anyhow!("hook stdin is not valid UTF-8: {error}"))
            });
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
    fn bounded_hook_input_accepts_exact_limit_and_rejects_one_byte_over() {
        let exact = vec![b'a'; 64];
        let accepted = read_bounded_hook_input(&mut exact.as_slice(), 64).expect("exact limit");
        assert_eq!(accepted.len(), 64);

        let over = vec![b'a'; 65];
        let error = read_bounded_hook_input(&mut over.as_slice(), 64)
            .unwrap_err()
            .to_string();
        assert!(error.contains("limit=64 bytes"));
        assert!(!error.contains("aaaa"));
    }
}
