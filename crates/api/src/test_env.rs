//! Serialized, self-restoring env var mutation for unit tests in this crate.
//!
//! - **Same process (cargo test threads):** [`tokio::sync::Mutex`] serializes holders.
//! - **Cross process (cargo-nextest):** a file lock under the system temp dir serializes holders
//!   so parallel nextest workers cannot clobber process env.
//!
//! Any test that sets or removes a process env var must hold an [`EnvGuard`] for its whole
//! duration. Values are restored on drop (also on panic).
//! Same file in each crate that mutates env in tests (`api`, `cli`, `data`, `execution`).
#![allow(dead_code)]

use fslock::LockFile;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use tokio::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::const_new(());

fn env_lock_path() -> PathBuf {
    std::env::temp_dir().join("clmm_lp_test_env.lock")
}

fn acquire_file_lock() -> LockFile {
    let path = env_lock_path();
    let mut file = LockFile::open(&path).unwrap_or_else(|e| {
        panic!("open {}: {e}", path.display());
    });
    file.lock().unwrap_or_else(|e| {
        panic!("lock {}: {e}", path.display());
    });
    file
}

pub(crate) struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
    _file: LockFile,
}

impl EnvGuard {
    /// For `#[tokio::test]` (holding the guard across `.await` is fine).
    pub(crate) async fn lock() -> Self {
        let mutex = ENV_LOCK.lock().await;
        let file = tokio::task::spawn_blocking(acquire_file_lock)
            .await
            .expect("join env file lock");
        Self::from_parts(mutex, file)
    }

    /// For plain `#[test]`; panics if called inside a Tokio runtime.
    pub(crate) fn blocking_lock() -> Self {
        let mutex = ENV_LOCK.blocking_lock();
        let file = acquire_file_lock();
        Self::from_parts(mutex, file)
    }

    fn from_parts(lock: MutexGuard<'static, ()>, file: LockFile) -> Self {
        Self {
            saved: Vec::new(),
            _lock: lock,
            _file: file,
        }
    }

    fn save(&mut self, key: &'static str) {
        if !self.saved.iter().any(|(k, _)| *k == key) {
            self.saved.push((key, std::env::var_os(key)));
        }
    }

    pub(crate) fn set(&mut self, key: &'static str, value: impl AsRef<OsStr>) {
        self.save(key);
        // SAFETY: all env mutation in this crate's tests is serialized by ENV_LOCK + file lock.
        unsafe { std::env::set_var(key, value) };
    }

    pub(crate) fn remove(&mut self, key: &'static str) {
        self.save(key);
        // SAFETY: all env mutation in this crate's tests is serialized by ENV_LOCK + file lock.
        unsafe { std::env::remove_var(key) };
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prev) in self.saved.drain(..).rev() {
            // SAFETY: still holding ENV_LOCK + file lock.
            unsafe {
                match prev {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}
