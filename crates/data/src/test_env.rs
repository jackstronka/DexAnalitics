//! Serialized, self-restoring env var mutation for unit tests in this crate.
//!
//! Tests run in parallel threads of one process; any test that sets or removes a process env var
//! must hold an [`EnvGuard`] for its whole duration. Values are restored on drop (also on panic).
//! Same file in each crate that mutates env in tests (`api`, `cli`, `data`, `execution`).
#![allow(dead_code)]

use std::ffi::{OsStr, OsString};
use tokio::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::const_new(());

pub(crate) struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvGuard {
    /// For `#[tokio::test]` (holding the guard across `.await` is fine).
    pub(crate) async fn lock() -> Self {
        Self::from_lock(ENV_LOCK.lock().await)
    }

    /// For plain `#[test]`; panics if called inside a Tokio runtime.
    pub(crate) fn blocking_lock() -> Self {
        Self::from_lock(ENV_LOCK.blocking_lock())
    }

    fn from_lock(lock: MutexGuard<'static, ()>) -> Self {
        Self {
            saved: Vec::new(),
            _lock: lock,
        }
    }

    fn save(&mut self, key: &'static str) {
        if !self.saved.iter().any(|(k, _)| *k == key) {
            self.saved.push((key, std::env::var_os(key)));
        }
    }

    pub(crate) fn set(&mut self, key: &'static str, value: impl AsRef<OsStr>) {
        self.save(key);
        // SAFETY: all env mutation in this crate's tests is serialized by ENV_LOCK.
        unsafe { std::env::set_var(key, value) };
    }

    pub(crate) fn remove(&mut self, key: &'static str) {
        self.save(key);
        // SAFETY: all env mutation in this crate's tests is serialized by ENV_LOCK.
        unsafe { std::env::remove_var(key) };
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prev) in self.saved.drain(..).rev() {
            // SAFETY: still holding ENV_LOCK.
            unsafe {
                match prev {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}
