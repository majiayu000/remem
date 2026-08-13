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
    if stdin_is_terminal {
        return Ok(None);
    }

    #[cfg(unix)]
    {
        read_fd_until_eof_or_timeout(std::io::stdin().as_raw_fd_for_hook(), timeout_ms)
    }
    #[cfg(not(unix))]
    {
        read_stdin_with_timeout_threaded(timeout_ms)
    }
}

#[cfg(unix)]
trait StdinRawFd {
    fn as_raw_fd_for_hook(&self) -> std::os::fd::RawFd;
}

#[cfg(unix)]
impl StdinRawFd for std::io::Stdin {
    fn as_raw_fd_for_hook(&self) -> std::os::fd::RawFd {
        std::os::fd::AsRawFd::as_raw_fd(self)
    }
}

#[cfg(unix)]
fn read_fd_until_eof_or_timeout(fd: std::os::fd::RawFd, timeout_ms: u64) -> Result<Option<String>> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags < 0 {
        return Err(anyhow!(
            "hook stdin fcntl failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(anyhow!(
            "hook stdin fcntl setfl failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    struct RestoreFlags {
        fd: std::os::fd::RawFd,
        flags: libc::c_int,
    }
    impl Drop for RestoreFlags {
        fn drop(&mut self) {
            unsafe {
                libc::fcntl(self.fd, libc::F_SETFL, self.flags);
            }
        }
    }
    let _restore = RestoreFlags { fd, flags };
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    let mut buffer = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(anyhow!("hook stdin timed out (limit={timeout_ms}ms)"));
        }
        let wait_ms = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
        let mut fds = [libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        }];
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 1, wait_ms) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(anyhow!("hook stdin poll failed: {err}"));
        }
        if rc == 0 {
            return Err(anyhow!("hook stdin timed out (limit={timeout_ms}ms)"));
        }
        let mut chunk = [0_u8; 8192];
        let n = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock
                || err.kind() == std::io::ErrorKind::Interrupted
            {
                continue;
            }
            return Err(anyhow!("hook stdin read failed: {err}"));
        }
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n as usize]);
        if buffer.len() > HOOK_STDIN_MAX_BYTES {
            return Err(anyhow!(
                "hook stdin exceeds configured bound (limit={HOOK_STDIN_MAX_BYTES} bytes)"
            ));
        }
    }
    bytes_to_optional_string(buffer)
}

fn bytes_to_optional_string(buffer: Vec<u8>) -> Result<Option<String>> {
    if buffer.is_empty() {
        return Ok(None);
    }
    let input = String::from_utf8(buffer)
        .map_err(|error| anyhow!("hook stdin is not valid UTF-8: {error}"))?;
    if input.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(input))
    }
}

#[cfg(not(unix))]
fn read_stdin_with_timeout_threaded(timeout_ms: u64) -> Result<Option<String>> {
    use std::sync::mpsc;
    use std::time::Duration;

    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let input = read_bounded_hook_input(&mut std::io::stdin(), HOOK_STDIN_MAX_BYTES).and_then(
            |bytes| {
                String::from_utf8(bytes)
                    .map_err(|error| anyhow!("hook stdin is not valid UTF-8: {error}"))
            },
        );
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
        Err(mpsc::RecvTimeoutError::Timeout) => {
            Err(anyhow!("hook stdin timed out (limit={timeout_ms}ms)"))
        }
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

    #[cfg(unix)]
    #[test]
    fn unix_poll_timeout_without_writer_fails_closed() -> Result<()> {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let started = std::time::Instant::now();
        let error = read_fd_until_eof_or_timeout(fds[0], 50)
            .expect_err("timeout should fail closed")
            .to_string();
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
        assert!(error.contains("timed out"));
        assert!(started.elapsed() < std::time::Duration::from_millis(400));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unix_poll_reads_payload_then_eof() -> Result<()> {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let payload = b"{\"cwd\":\"/tmp\"}";
        assert_eq!(
            unsafe { libc::write(fds[1], payload.as_ptr().cast(), payload.len()) },
            payload.len() as isize
        );
        unsafe { libc::close(fds[1]) };
        let input = read_fd_until_eof_or_timeout(fds[0], 1000)?;
        unsafe { libc::close(fds[0]) };
        assert_eq!(input.as_deref(), Some("{\"cwd\":\"/tmp\"}"));
        Ok(())
    }
}
