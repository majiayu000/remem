use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use anyhow::{Context, Result};

use super::build_unauthenticated_download_api;

#[test]
fn download_client_does_not_send_parent_cache_token() -> Result<()> {
    let root = std::env::temp_dir().join(format!(
        "remem-download-token-test-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let cache_dir = root.join(".remem-download-staging");
    std::fs::create_dir_all(&cache_dir)?;
    let secret = root.join("secret");
    std::fs::write(&secret, b"must-not-become-an-authorization-header")?;
    let token = root.join("token");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&secret, &token)?;
    #[cfg(not(unix))]
    std::fs::write(&token, std::fs::read(&secret)?)?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server = std::thread::spawn(move || -> Result<Vec<u8>> {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let read = stream.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
        stream.write_all(
            b"HTTP/1.1 206 Partial Content\r\n\
              Content-Length: 1\r\n\
              Content-Range: bytes 0-0/1\r\n\
              ETag: \"test-etag\"\r\n\
              X-Repo-Commit: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n\
              Connection: close\r\n\r\nx",
        )?;
        Ok(request)
    });

    let endpoint = format!("http://{address}");
    let api = build_unauthenticated_download_api(&cache_dir, &endpoint)?;
    let metadata = api
        .metadata(&format!("{endpoint}/probe"))
        .context("send token-isolation probe")?;
    assert_eq!(metadata.commit_hash(), "a".repeat(40));
    let request = server
        .join()
        .map_err(|_| anyhow::anyhow!("token-isolation server panicked"))??;
    let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
    assert!(!request.contains("authorization:"), "{request}");
    assert!(!request.contains("must-not-become"), "{request}");

    std::fs::remove_dir_all(&root)?;
    Ok(())
}
