use anyhow::Result;
use std::io::IsTerminal;

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
        let input = std::io::read_to_string(std::io::stdin());
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
        Ok(Err(error)) => Err(error.into()),
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
}
