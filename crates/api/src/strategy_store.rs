//! Persistence layer for strategies configured via HTTP.
//!
//! Goal: strategies should survive API restarts until explicitly deleted.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_STORE_REL_PATH: &str = "data/strategies.json";
const STORE_VERSION: u32 = 1;

/// Strategy record persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedStrategy {
    pub id: String,
    pub name: String,
    pub config: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StrategyStoreFile {
    pub version: u32,
    pub strategies: Vec<PersistedStrategy>,
}

fn repo_root() -> Option<PathBuf> {
    std::env::var("CLMM_REPO_ROOT")
        .ok()
        .and_then(|s| {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                Some(PathBuf::from(s))
            }
        })
}

/// Returns whether persistence is enabled.
///
/// Disabled by default in unit/integration tests to avoid mutating the dev workspace.
pub fn enabled() -> bool {
    if cfg!(test) {
        return false;
    }
    match std::env::var("CLMM_PERSIST_STRATEGIES") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "n" | "off"
        ),
        Err(_) => true,
    }
}

/// Where strategies are stored on disk.
pub fn store_path() -> PathBuf {
    if let Ok(p) = std::env::var("CLMM_STRATEGY_STORE_PATH") {
        let p = p.trim().to_string();
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }

    if let Some(root) = repo_root() {
        root.join(DEFAULT_STORE_REL_PATH)
    } else {
        PathBuf::from(DEFAULT_STORE_REL_PATH)
    }
}

/// Loads persisted strategies from disk.
pub fn try_load_persisted_strategies() -> io::Result<Vec<PersistedStrategy>> {
    if !enabled() {
        return Ok(vec![]);
    }

    let path = store_path();
    if !path.exists() {
        return Ok(vec![]);
    }

    let s = fs::read_to_string(&path)?;
    let parsed: StrategyStoreFile = serde_json::from_str(&s).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse strategies store JSON: {e}"),
        )
    })?;

    if parsed.version != STORE_VERSION {
        return Ok(vec![]);
    }

    Ok(parsed.strategies)
}

/// Saves persisted strategies to disk (atomic best-effort).
pub fn try_save_persisted_strategies(
    strategies: &[PersistedStrategy],
) -> io::Result<()> {
    if !enabled() {
        return Ok(());
    }

    let path = store_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let file_name = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("strategies.json");
        let tmp_name = format!("{file_name}.tmp.{now_ms}");
        match path.parent() {
            Some(parent) => parent.join(tmp_name),
            None => PathBuf::from(tmp_name),
        }
    };

    let store = StrategyStoreFile {
        version: STORE_VERSION,
        strategies: strategies.to_vec(),
    };

    let json = serde_json::to_string(&store).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to serialize strategies store JSON: {e}"),
        )
    })?;

    fs::write(&tmp_path, json)?;

    // Replace target.
    if path.exists() {
        let _ = fs::remove_file(&path);
    }
    fs::rename(&tmp_path, &path)?;
    Ok(())
}

