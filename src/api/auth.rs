use anyhow::{anyhow, Context, Result};
use axum::{
    extract::Request,
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{io::Write, path::PathBuf};

use super::helpers::error_response;

const API_TOKEN_FILE: &str = ".api-token";
const API_TOKEN_BYTES: usize = 32;

pub(crate) fn api_token_path() -> PathBuf {
    crate::db::data_dir().join(API_TOKEN_FILE)
}

/// Ensure the REST API bearer token exists and return its path.
///
/// Call this before using [`crate::api::build_router`] directly. The
/// `remem api` command calls it automatically.
pub fn ensure_api_token() -> Result<PathBuf> {
    let path = api_token_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create API token parent {}", parent.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).with_context(
                || format!("set API token parent permissions {}", parent.display()),
            )?;
        }
    }

    if path.exists() {
        validate_existing_token(&path)?;
        enforce_token_permissions(&path)?;
        return Ok(path);
    }

    let token = generate_api_token()?;
    write_new_token(&path, &token)?;
    Ok(path)
}

/// Load the REST API bearer token from the remem data directory.
pub fn load_api_token() -> Result<String> {
    let path = api_token_path();
    let token = std::fs::read_to_string(&path)
        .with_context(|| format!("read API token from {}", path.display()))?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("API token file is empty: {}", path.display()));
    }
    Ok(token)
}

pub(in crate::api) async fn require_api_token(req: Request, next: Next) -> Response {
    let expected = match load_api_token() {
        Ok(token) => token,
        Err(err) => {
            crate::log::error("api", &format!("API token unavailable: {err}"));
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_token_unavailable",
                "API token is not configured",
            )
            .into_response();
        }
    };

    if request_has_token(req.headers(), &expected) {
        next.run(req).await
    } else {
        error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing or invalid API token",
        )
        .into_response()
    }
}

fn validate_existing_token(path: &std::path::Path) -> Result<()> {
    let token = std::fs::read_to_string(path)
        .with_context(|| format!("read existing API token {}", path.display()))?;
    if token.trim().is_empty() {
        return Err(anyhow!("existing API token is empty: {}", path.display()));
    }
    Ok(())
}

fn generate_api_token() -> Result<String> {
    let mut bytes = [0u8; API_TOKEN_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|err| anyhow!("OS randomness unavailable while generating API token: {err}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn write_new_token(path: &std::path::Path, token: &str) -> Result<()> {
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        match std::fs::OpenOptions::new()
            .mode(0o600)
            .create_new(true)
            .write(true)
            .open(path)
        {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                validate_existing_token(path)?;
                enforce_token_permissions(path)?;
                return Ok(());
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("create API token file {}", path.display()));
            }
        }
    };

    #[cfg(not(unix))]
    let mut file = match std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            validate_existing_token(path)?;
            enforce_token_permissions(path)?;
            return Ok(());
        }
        Err(err) => {
            return Err(err).with_context(|| format!("create API token file {}", path.display()));
        }
    };

    if let Err(err) = file.write_all(token.as_bytes()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(anyhow!(
            "write API token file {} failed: {}",
            path.display(),
            err
        ));
    }
    enforce_token_permissions(path)?;
    Ok(())
}

fn enforce_token_permissions(path: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("set API token permissions {}", path.display()))?;
    }
    Ok(())
}

fn request_has_token(headers: &HeaderMap, expected: &str) -> bool {
    let Some(actual) = bearer_token(headers) else {
        return false;
    };
    constant_time_eq(actual.as_bytes(), expected.as_bytes())
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?;
    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

fn constant_time_eq(actual: &[u8], expected: &[u8]) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in actual.iter().zip(expected.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::{constant_time_eq, ensure_api_token, load_api_token};
    use crate::db::test_support::ScopedTestDataDir;

    #[test]
    fn ensure_api_token_creates_and_preserves_token() {
        let data_dir = ScopedTestDataDir::new("api-token");

        let path = ensure_api_token().expect("token should be created");
        assert_eq!(path, data_dir.path.join(".api-token"));
        let first = load_api_token().expect("token should load");
        assert_eq!(first.len(), 64);

        let second_path = ensure_api_token().expect("existing token should be reused");
        let second = load_api_token().expect("token should load again");
        assert_eq!(second_path, path);
        assert_eq!(second, first);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .expect("token metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn constant_time_eq_requires_exact_match() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
