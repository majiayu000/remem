use std::ffi::OsString;
use std::sync::{Mutex, OnceLock};

fn log_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) fn with_log_envs<T>(vars: &[(&'static str, Option<&str>)], f: impl FnOnce() -> T) -> T {
    let _guard = log_env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = ScopedLogEnv::set(vars);
    f()
}

struct ScopedLogEnv {
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl ScopedLogEnv {
    fn set(vars: &[(&'static str, Option<&str>)]) -> Self {
        let previous = vars
            .iter()
            .map(|(name, _)| (*name, std::env::var_os(name)))
            .collect::<Vec<_>>();

        for (name, value) in vars {
            match value {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
        }

        Self { previous }
    }
}

impl Drop for ScopedLogEnv {
    fn drop(&mut self) {
        for (name, value) in self.previous.drain(..) {
            match value {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
        }
    }
}
