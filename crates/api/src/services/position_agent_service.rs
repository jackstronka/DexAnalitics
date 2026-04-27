//! Lightweight per-position agent session + chat persistence.

use crate::error::{ApiError, ApiResult};
use crate::models::{
    AgentChatMessage, AgentPositionSession, AgentScanResponse, AgentWorkerSettings, AgentWorkerStatus,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use uuid::Uuid;

static AGENT_STATE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAgentState {
    sessions: Vec<AgentPositionSession>,
    messages: Vec<AgentChatMessage>,
}

impl StoredAgentState {
    fn new() -> Self {
        Self {
            sessions: Vec::new(),
            messages: Vec::new(),
        }
    }
}

fn storage_path() -> PathBuf {
    agent_data_root().join("position_agent_state.json")
}

fn settings_path() -> PathBuf {
    agent_data_root().join("agent_worker_settings.json")
}

fn status_path() -> PathBuf {
    agent_data_root().join("agent_worker_status.json")
}

fn events_path() -> PathBuf {
    agent_data_root().join("position_agent_events.jsonl")
}

fn agent_data_root() -> PathBuf {
    std::env::var("CLMM_AGENT_DATA_DIR")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data").join("agent"))
}

fn ensure_parent_dir(path: &Path) -> ApiResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| ApiError::internal(format!("create agent data dir: {e}")))?;
    }
    Ok(())
}

fn load_state() -> ApiResult<StoredAgentState> {
    let path = storage_path();
    if !path.exists() {
        return Ok(StoredAgentState::new());
    }
    let txt = fs::read_to_string(&path).map_err(|e| ApiError::internal(format!("read agent state: {e}")))?;
    if txt.trim().is_empty() {
        return Ok(StoredAgentState::new());
    }
    serde_json::from_str(&txt).map_err(|e| ApiError::internal(format!("parse agent state json: {e}")))
}

fn save_state(state: &StoredAgentState) -> ApiResult<()> {
    let path = storage_path();
    ensure_parent_dir(&path)?;
    let body = serde_json::to_string_pretty(state)
        .map_err(|e| ApiError::internal(format!("serialize agent state: {e}")))?;
    fs::write(&path, body).map_err(|e| ApiError::internal(format!("write agent state: {e}")))?;
    Ok(())
}

fn with_state_lock<T>(f: impl FnOnce() -> ApiResult<T>) -> ApiResult<T> {
    let _guard = AGENT_STATE_LOCK
        .lock()
        .map_err(|_| ApiError::internal("agent state lock poisoned"))?;
    f()
}

fn append_event_log(line: &str) -> ApiResult<()> {
    let path = events_path();
    ensure_parent_dir(&path)?;
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| ApiError::internal(format!("open agent events jsonl: {e}")))?;
    writeln!(f, "{line}").map_err(|e| ApiError::internal(format!("append agent events jsonl: {e}")))?;
    Ok(())
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

pub fn load_worker_settings() -> ApiResult<AgentWorkerSettings> {
    let path = settings_path();
    if !path.exists() {
        return Ok(AgentWorkerSettings::default());
    }
    let txt =
        fs::read_to_string(&path).map_err(|e| ApiError::internal(format!("read worker settings: {e}")))?;
    if txt.trim().is_empty() {
        return Ok(AgentWorkerSettings::default());
    }
    let mut settings: AgentWorkerSettings = serde_json::from_str(&txt)
        .map_err(|e| ApiError::internal(format!("parse worker settings json: {e}")))?;
    settings.default_position_scan_interval_hours = settings.default_position_scan_interval_hours.max(1);
    settings.cross_pair_scan_interval_hours = settings.cross_pair_scan_interval_hours.max(1);
    Ok(settings)
}

pub fn save_worker_settings(settings: &AgentWorkerSettings) -> ApiResult<()> {
    let path = settings_path();
    ensure_parent_dir(&path)?;
    let body = serde_json::to_string_pretty(settings)
        .map_err(|e| ApiError::internal(format!("serialize worker settings: {e}")))?;
    fs::write(&path, body).map_err(|e| ApiError::internal(format!("write worker settings: {e}")))?;
    Ok(())
}

pub fn load_worker_status() -> ApiResult<AgentWorkerStatus> {
    let path = status_path();
    if !path.exists() {
        return Ok(AgentWorkerStatus {
            last_tick_ts_utc: None,
            ticks_total: 0,
            scanned_positions_total: 0,
            scanned_positions_last_tick: 0,
            last_error: None,
        });
    }
    let txt =
        fs::read_to_string(&path).map_err(|e| ApiError::internal(format!("read worker status: {e}")))?;
    if txt.trim().is_empty() {
        return Ok(AgentWorkerStatus {
            last_tick_ts_utc: None,
            ticks_total: 0,
            scanned_positions_total: 0,
            scanned_positions_last_tick: 0,
            last_error: None,
        });
    }
    serde_json::from_str(&txt).map_err(|e| ApiError::internal(format!("parse worker status json: {e}")))
}

fn save_worker_status(status: &AgentWorkerStatus) -> ApiResult<()> {
    let path = status_path();
    ensure_parent_dir(&path)?;
    let body = serde_json::to_string_pretty(status)
        .map_err(|e| ApiError::internal(format!("serialize worker status: {e}")))?;
    fs::write(&path, body).map_err(|e| ApiError::internal(format!("write worker status: {e}")))?;
    Ok(())
}

pub fn get_or_create_session(
    position_address: &str,
    scan_interval_hours: Option<u64>,
) -> ApiResult<AgentPositionSession> {
    with_state_lock(|| {
        let mut state = load_state()?;
        if let Some(existing) = state
            .sessions
            .iter()
            .find(|s| s.position_address == position_address)
            .cloned()
        {
            return Ok(existing);
        }
        let started = now_rfc3339();
        let default_interval = load_worker_settings()?.default_position_scan_interval_hours.max(1);
        let interval = scan_interval_hours.unwrap_or(default_interval).max(1);
        let next_scan = (Utc::now() + Duration::hours(interval as i64)).to_rfc3339();
        let session = AgentPositionSession {
            position_address: position_address.to_string(),
            status: "active".to_string(),
            started_ts_utc: started,
            last_scan_ts_utc: None,
            next_scan_ts_utc: Some(next_scan),
            scan_interval_hours: interval,
        };
        state.sessions.push(session.clone());
        save_state(&state)?;
        Ok(session)
    })
}

pub fn list_chat(position_address: &str) -> ApiResult<(Option<AgentPositionSession>, Vec<AgentChatMessage>)> {
    with_state_lock(|| {
        let state = load_state()?;
        let session = state
            .sessions
            .into_iter()
            .find(|s| s.position_address == position_address);
        let mut messages: Vec<AgentChatMessage> = state
            .messages
            .into_iter()
            .filter(|m| m.position_address == position_address)
            .collect();
        messages.sort_by(|a, b| a.ts_utc.cmp(&b.ts_utc));
        Ok((session, messages))
    })
}

pub fn append_message(
    position_address: &str,
    role: &str,
    kind: &str,
    content: String,
) -> ApiResult<AgentChatMessage> {
    let message = with_state_lock(|| {
        let mut state = load_state()?;
        let message = AgentChatMessage {
            id: format!("msg-{}", Uuid::new_v4()),
            position_address: position_address.to_string(),
            ts_utc: now_rfc3339(),
            role: role.to_string(),
            kind: kind.to_string(),
            content,
        };
        state.messages.push(message.clone());
        save_state(&state)?;
        Ok(message)
    })?;
    let serialized = serde_json::to_string(&message)
        .map_err(|e| ApiError::internal(format!("serialize agent event line: {e}")))?;
    append_event_log(&serialized)?;
    Ok(message)
}

pub fn touch_scan(position_address: &str) -> ApiResult<AgentPositionSession> {
    with_state_lock(|| {
        let mut state = load_state()?;
        let now = Utc::now();
        let mut found = None;
        for s in &mut state.sessions {
            if s.position_address == position_address {
                s.last_scan_ts_utc = Some(now.to_rfc3339());
                let interval = s.scan_interval_hours.max(1);
                s.next_scan_ts_utc = Some((now + Duration::hours(interval as i64)).to_rfc3339());
                found = Some(s.clone());
                break;
            }
        }
        let Some(updated) = found else {
            return Err(ApiError::not_found("Agent session not found for this position"));
        };
        save_state(&state)?;
        Ok(updated)
    })
}

pub fn scan_recommendations(
    position_address: &str,
    include_cross_pair_scan: bool,
) -> ApiResult<AgentScanResponse> {
    let mut recommendations = vec![
        "Porownaj zakres z oknem 7d: przetestuj warianty +/-1.5%, +/-2.5%, +/-4.0% i wybierz najwyzszy fee trend przy akceptowalnej liczbie rebalance.".to_string(),
        "Dla tej pozycji uruchom test scenariusza 'narrow vs wide': wezszy zakres zwykle podnosi fee, ale podnosi tez ryzyko wypadania poza range.".to_string(),
    ];
    if include_cross_pair_scan {
        recommendations.push(
            "Skan cross-pair (domyslnie co 4h): porownaj min. 3 pary i przedstaw 2-3 propozycje alokacji kapitalu (conservative/balanced/aggressive).".to_string(),
        );
    }
    let session = touch_scan(position_address)?;
    Ok(AgentScanResponse {
        position_address: position_address.to_string(),
        scanned_ts_utc: now_rfc3339(),
        include_cross_pair_scan,
        recommendations,
        session,
    })
}

fn parse_ts(ts: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|x| x.with_timezone(&Utc))
}

pub fn due_sessions(now: DateTime<Utc>) -> ApiResult<Vec<AgentPositionSession>> {
    with_state_lock(|| {
        let state = load_state()?;
        let due = state
            .sessions
            .into_iter()
            .filter(|s| s.status == "active")
            .filter(|s| match s.next_scan_ts_utc.as_deref().and_then(parse_ts) {
                Some(next) => next <= now,
                None => true,
            })
            .collect();
        Ok(due)
    })
}

pub fn run_periodic_scan_tick() -> ApiResult<usize> {
    let mut status = load_worker_status()?;
    status.ticks_total = status.ticks_total.saturating_add(1);
    status.last_tick_ts_utc = Some(now_rfc3339());
    let result = (|| -> ApiResult<usize> {
        let settings = load_worker_settings()?;
        if !settings.enabled {
            return Ok(0);
        }
        let now = Utc::now();
        let sessions = due_sessions(now)?;
        let mut scanned = 0usize;
        for s in sessions {
            let scan = scan_recommendations(&s.position_address, settings.include_cross_pair_scan)?;
            for rec in scan.recommendations {
                let _ = append_message(&s.position_address, "agent", "insight", rec)?;
            }
            scanned += 1;
        }
        Ok(scanned)
    })();
    match result {
        Ok(scanned) => {
            status.last_error = None;
            status.scanned_positions_last_tick = scanned as u64;
            status.scanned_positions_total = status.scanned_positions_total.saturating_add(scanned as u64);
            save_worker_status(&status)?;
            Ok(scanned)
        }
        Err(e) => {
            status.last_error = Some(e.to_string());
            status.scanned_positions_last_tick = 0;
            save_worker_status(&status)?;
            Err(e)
        }
    }
}

#[allow(dead_code)]
fn _read_last_events(limit: usize) -> ApiResult<Vec<String>> {
    let path = events_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let f = fs::File::open(path).map_err(|e| ApiError::internal(format!("open agent events: {e}")))?;
    let reader = BufReader::new(f);
    let mut lines: Vec<String> = reader.lines().map_while(Result::ok).collect();
    if lines.len() > limit {
        lines = lines[lines.len() - limit..].to_vec();
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex};
    use tempfile::tempdir;

    static TEST_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn append_message_concurrent_keeps_all_rows() {
        let _guard = TEST_ENV_LOCK.lock().expect("test env lock");
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("agent-data");
        // SAFETY: guarded by TEST_ENV_LOCK to avoid concurrent env var mutation in tests.
        unsafe { std::env::set_var("CLMM_AGENT_DATA_DIR", &path) };

        let position = "7Mxt4r3kquyMwxPjggwYV4XeY2vQyxuN8LwfbYQj1m8x";
        let threads = 24usize;
        let mut joins = Vec::new();
        for i in 0..threads {
            let content = format!("message-{i}");
            joins.push(std::thread::spawn(move || {
                append_message(position, "user", "question", content)
                    .expect("append message should succeed");
            }));
        }
        for j in joins {
            j.join().expect("thread should not panic");
        }

        let (_session, messages) = list_chat(position).expect("list chat should succeed");
        assert_eq!(messages.len(), threads);

        let mut uniq = std::collections::BTreeSet::new();
        for m in messages {
            assert!(uniq.insert(m.id), "message id collision detected");
        }

        // SAFETY: guarded by TEST_ENV_LOCK to avoid concurrent env var mutation in tests.
        unsafe { std::env::remove_var("CLMM_AGENT_DATA_DIR") };
    }

    #[test]
    fn get_or_create_session_concurrent_creates_single_session() {
        let _guard = TEST_ENV_LOCK.lock().expect("test env lock");
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("agent-data");
        // SAFETY: guarded by TEST_ENV_LOCK to avoid concurrent env var mutation in tests.
        unsafe { std::env::set_var("CLMM_AGENT_DATA_DIR", &path) };

        let position = "6z4o9M1zN8vnM9x4fXkzQz3wN5sYj2Yy1W4Kx9Yq3FJ2";
        let threads = 20usize;
        let mut joins = Vec::new();
        for _ in 0..threads {
            joins.push(std::thread::spawn(move || {
                get_or_create_session(position, Some(4)).expect("session create should succeed")
            }));
        }

        let mut out = Vec::new();
        for j in joins {
            out.push(j.join().expect("thread should not panic"));
        }
        assert_eq!(out.len(), threads);
        assert!(out.iter().all(|s| s.position_address == position));

        let (session, _messages) = list_chat(position).expect("list chat should succeed");
        let sess = session.expect("session should exist");
        assert_eq!(sess.position_address, position);

        let state = load_state().expect("state should load");
        let count = state
            .sessions
            .iter()
            .filter(|s| s.position_address == position)
            .count();
        assert_eq!(count, 1, "session should not be duplicated");

        // SAFETY: guarded by TEST_ENV_LOCK to avoid concurrent env var mutation in tests.
        unsafe { std::env::remove_var("CLMM_AGENT_DATA_DIR") };
    }
}
