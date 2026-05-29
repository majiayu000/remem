use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct TempDataDir {
    _guard: MutexGuard<'static, ()>,
    previous: Option<OsString>,
    path: PathBuf,
}

impl TempDataDir {
    fn new(label: &str) -> Self {
        let guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("REMEM_DATA_DIR");
        let path = std::env::temp_dir().join(format!(
            "remem-integration-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::env::set_var("REMEM_DATA_DIR", &path);
        Self {
            _guard: guard,
            previous,
            path,
        }
    }
}

impl Drop for TempDataDir {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var("REMEM_DATA_DIR", previous);
        } else {
            std::env::remove_var("REMEM_DATA_DIR");
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn public_api_token_helpers_prepare_router_auth() {
    let _data_dir = TempDataDir::new("public-api-token");

    let token_path = remem::api::ensure_api_token().expect("token setup should succeed");
    let token = remem::api::load_api_token().expect("token should load");

    assert!(token_path.ends_with(".api-token"));
    assert_eq!(token.len(), 64);
}
