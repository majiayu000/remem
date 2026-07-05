use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Condvar, Mutex};
use std::thread::ThreadId;

#[derive(Debug)]
pub(crate) struct EnvLockError;

impl std::fmt::Display for EnvLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("environment lock failed")
    }
}

impl std::error::Error for EnvLockError {}

#[derive(Debug)]
struct EnvLockState {
    owner: Option<ThreadId>,
    depth: usize,
}

pub(crate) struct EnvLock {
    state: Mutex<EnvLockState>,
    available: Condvar,
}

pub(crate) struct EnvGuard {
    lock: &'static EnvLock,
    _not_send: PhantomData<Rc<()>>,
}

impl EnvLock {
    pub(crate) const fn new() -> Self {
        Self {
            state: Mutex::new(EnvLockState {
                owner: None,
                depth: 0,
            }),
            available: Condvar::new(),
        }
    }

    pub(crate) fn lock(&'static self) -> Result<EnvGuard, EnvLockError> {
        let current = std::thread::current().id();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            match state.owner {
                None => {
                    state.owner = Some(current);
                    state.depth = 1;
                    return Ok(EnvGuard {
                        lock: self,
                        _not_send: PhantomData,
                    });
                }
                Some(owner) if owner == current => {
                    state.depth += 1;
                    return Ok(EnvGuard {
                        lock: self,
                        _not_send: PhantomData,
                    });
                }
                Some(_) => {
                    state = self
                        .available
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            }
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        let current = std::thread::current().id();
        let mut state = self
            .lock
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        debug_assert_eq!(state.owner, Some(current));
        state.depth = state.depth.saturating_sub(1);
        if state.depth == 0 {
            state.owner = None;
            self.lock.available.notify_all();
        }
    }
}
