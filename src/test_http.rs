//! Bounded HTTP request reader for local test servers.
use std::io::{self, Read};

pub(crate) fn read_request(reader: &mut impl Read) -> io::Result<String> {
    const LIMIT: usize = 64 * 1024;
    let mut request = Vec::new();
    let mut expected = None;
    loop {
        let mut chunk = [0_u8; 1024];
        let count = match reader.read(&mut chunk) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete test HTTP request",
            ));
        }
        request.extend_from_slice(&chunk[..count]);
        if request.len() > LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "test request too large",
            ));
        }
        if expected.is_none() {
            if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = std::str::from_utf8(&request[..end])
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                let length = headers
                    .split("\r\n")
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map_or("0", |(_, value)| value.trim())
                    .parse::<usize>()
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                let total = end
                    .checked_add(4)
                    .and_then(|value| value.checked_add(length))
                    .filter(|total| *total <= LIMIT)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "test request too large")
                    })?;
                expected = Some(total);
            }
        }
        if let Some(total) = expected {
            if request.len() >= total {
                request.truncate(total);
                return String::from_utf8(request)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
            }
        }
    }
}

#[test]
fn reads_fragmented_headers_and_body() {
    struct Fragments<'a> {
        bytes: &'a [u8],
        limit: usize,
    }
    impl Read for Fragments<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let limit = self.limit.min(buffer.len());
            self.bytes.read(&mut buffer[..limit])
        }
    }
    let body = r#"{"model":"requested-model","input":"hello猫"}"#;
    let request = format!(
        "POST /v1/embeddings HTTP/1.1\r\nauthorization: Bearer fixture\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    for limit in 1..=request.len() {
        let mut reader = Fragments {
            bytes: request.as_bytes(),
            limit,
        };
        assert_eq!(
            read_request(&mut reader).unwrap(),
            request,
            "fragment size {limit}"
        );
    }
}

#[test]
fn rejects_truncated_body() {
    let mut request = &b"POST / HTTP/1.1\r\ncontent-length: 3\r\n\r\nx"[..];
    assert_eq!(
        read_request(&mut request).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
}
