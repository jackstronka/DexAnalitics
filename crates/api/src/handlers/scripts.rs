//! Tools scripts manifest, `script_runs.jsonl` tail index, and proxy to localhost runner.

use crate::error::{ApiError, ApiResult};
use crate::models::{RunScriptRequest, ScriptCatalogItem, ScriptRunRecord, ScriptsListResponse};
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
};
use reqwest::header;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path as StdPath, PathBuf};
use std::sync::LazyLock;

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        // The localhost script runner uses PowerShell HttpListener which is most reliable over HTTP/1.1.
        .http1_only()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .expect("reqwest client for script runner")
});

#[derive(Debug, Deserialize)]
struct ManifestFile {
    #[allow(dead_code)]
    schema_version: Option<u32>,
    scripts: Vec<ManifestScript>,
}

#[derive(Debug, Clone, Deserialize)]
struct ManifestScript {
    id: String,
    path: String,
    summary: String,
    #[serde(default)]
    when_to_use: Option<String>,
    #[serde(default)]
    risk: Option<String>,
    #[serde(default = "default_runnable")]
    runnable: bool,
    #[serde(default)]
    actions: Vec<String>,
}

fn default_runnable() -> bool {
    true
}

/// Szuka katalogu głównego repo (gdzie jest `tools/` i manifest albo layout workspace).
fn find_repo_root(start: &StdPath) -> Option<PathBuf> {
    let mut p = start.to_path_buf();
    for _ in 0..24 {
        let manifest = p.join("tools").join("scripts-manifest.json");
        if manifest.is_file() {
            return Some(p);
        }
        // Monorepo bez manifestu / zła ścieżka startu: `Cargo.toml` + `tools/` + `crates/`.
        if p.join("Cargo.toml").is_file() && p.join("tools").is_dir() && p.join("crates").is_dir() {
            return Some(p);
        }
        if !p.pop() {
            break;
        }
    }
    None
}

fn resolve_repo_root(state: &AppState) -> PathBuf {
    let explicit = state
        .config
        .repo_root
        .as_ref()
        .map(std::string::String::clone)
        .or_else(|| std::env::var("CLMM_REPO_ROOT").ok())
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from);
    if let Some(ref path) = explicit {
        return path.clone();
    }

    if let Ok(cwd) = std::env::current_dir() {
        if let Some(root) = find_repo_root(&cwd) {
            tracing::info!(
                cwd = %cwd.display(),
                repo_root = %root.display(),
                "scripts: resolved repo root (walk from cwd)"
            );
            return root;
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(PathBuf::from).unwrap_or_default();
        for _ in 0..28 {
            if let Some(root) = find_repo_root(&dir) {
                tracing::info!(
                    exe_dir = %dir.display(),
                    repo_root = %root.display(),
                    "scripts: resolved repo root (walk from executable)"
                );
                return root;
            }
            if !dir.pop() {
                break;
            }
        }
    }

    let fallback = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    tracing::warn!(
        cwd = %fallback.display(),
        "scripts: could not find repo root (no tools/scripts-manifest.json / workspace layout); set CLMM_REPO_ROOT"
    );
    fallback
}

fn read_manifest(repo: &StdPath) -> Result<ManifestFile, String> {
    let p = repo.join("tools").join("scripts-manifest.json");
    let raw = std::fs::read_to_string(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("parse manifest: {e}"))
}

fn normalize_repo_path(p: &str) -> String {
    p.replace('\\', "/")
}

/// Top-level `tools/*.ps1` (nie podkatalogi — tam nadal wymagany wpis w manifeście).
fn discover_top_level_ps1(repo: &StdPath) -> Vec<(String, String)> {
    let tools = repo.join("tools");
    let Ok(rd) = std::fs::read_dir(&tools) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        if p.extension().and_then(|x| x.to_str()) != Some("ps1") {
            continue;
        }
        let name = p.file_name().and_then(|x| x.to_str()).unwrap_or("");
        if name == "scripts-manifest.json" {
            continue;
        }
        let stem = p.file_stem().and_then(|x| x.to_str()).unwrap_or("");
        let rel = format!("tools/{name}");
        out.push((stem.to_string(), rel));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Wszystkie wpisy z manifestu + brakujące pliki `tools/*.ps1`. Drugi element: `true` = dopisany ze skanu dysku.
fn merge_manifest_with_disk(repo: &StdPath, mf: ManifestFile) -> Vec<(ManifestScript, bool)> {
    let mut manifest_paths: std::collections::HashSet<String> = mf
        .scripts
        .iter()
        .map(|s| normalize_repo_path(&s.path))
        .collect();
    let mut manifest_ids: std::collections::HashSet<String> =
        mf.scripts.iter().map(|s| s.id.clone()).collect();

    let mut out: Vec<(ManifestScript, bool)> = mf.scripts.into_iter().map(|s| (s, false)).collect();

    for (stem, rel) in discover_top_level_ps1(repo) {
        let norm = normalize_repo_path(&rel);
        if manifest_paths.contains(&norm) {
            continue;
        }
        if manifest_ids.contains(&stem) {
            continue;
        }
        manifest_paths.insert(norm);
        manifest_ids.insert(stem.clone());
        out.push((
            ManifestScript {
                id: stem,
                path: rel,
                summary: "Plik wykryty na dysku — brak wpisu w tools/scripts-manifest.json (dopisz opis w manifeście)."
                    .to_string(),
                when_to_use: Some(
                    "Uruchomienie i kopia komendy działają jak dla pozostałych skryptów.".to_string(),
                ),
                risk: Some("unknown".to_string()),
                runnable: true,
                actions: vec!["run".to_string(), "copy_command".to_string()],
            },
            true,
        ));
    }

    out.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    out
}

/// Gdy brak manifestu: pokaż wszystkie `tools/*.ps1` z katalogu głównego `tools/`.
fn catalog_disk_only(repo: &StdPath) -> Vec<ManifestScript> {
    let mut v: Vec<ManifestScript> = discover_top_level_ps1(repo)
        .into_iter()
        .map(|(stem, rel)| ManifestScript {
            id: stem.clone(),
            path: rel,
            summary: "Brak scripts-manifest.json — skrypt wykryty tylko z nazwy pliku.".to_string(),
            when_to_use: Some(
                "Utwórz tools/scripts-manifest.json z opisami (zalecane).".to_string(),
            ),
            risk: Some("unknown".to_string()),
            runnable: true,
            actions: vec!["run".to_string(), "copy_command".to_string()],
        })
        .collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
}

fn manifest_script_to_item(
    s: ManifestScript,
    last: &HashMap<String, ScriptRunRecord>,
    auto_discovered: bool,
) -> ScriptCatalogItem {
    let id = s.id.clone();
    ScriptCatalogItem {
        id: s.id,
        path: s.path,
        summary: s.summary,
        when_to_use: s.when_to_use,
        risk: s.risk,
        runnable: s.runnable,
        actions: s.actions,
        last_run: last.get(&id).cloned(),
        auto_discovered,
    }
}

/// Rozstrzyga skrypt do uruchomienia: najpierw manifest, potem `tools/{id}.ps1`.
fn resolve_script_for_run(repo: &StdPath, mf: &ManifestFile, id: &str) -> Option<ManifestScript> {
    if let Some(s) = mf.scripts.iter().find(|s| s.id == id) {
        return Some(s.clone());
    }
    let rel = format!("tools/{id}.ps1");
    let p = repo.join(&rel);
    if p.is_file() {
        return Some(ManifestScript {
            id: id.to_string(),
            path: rel,
            summary: "Auto-discovered script (manifest miss or no manifest).".to_string(),
            when_to_use: None,
            risk: Some("unknown".to_string()),
            runnable: true,
            actions: vec!["run".to_string(), "copy_command".to_string()],
        });
    }
    None
}

fn index_last_runs(path: &StdPath) -> HashMap<String, ScriptRunRecord> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let mut best: HashMap<String, (chrono::DateTime<chrono::Utc>, ScriptRunRecord)> =
        HashMap::new();
    for line in raw.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(rec) = serde_json::from_str::<ScriptRunRecord>(t) else {
            continue;
        };
        let ts = chrono::DateTime::parse_from_rfc3339(&rec.ts_utc)
            .ok()
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);
        let replace = match best.get(&rec.script_id) {
            None => true,
            Some((prev_ts, _)) => ts >= *prev_ts,
        };
        if replace {
            best.insert(rec.script_id.clone(), (ts, rec));
        }
    }
    best.into_iter().map(|(k, (_, v))| (k, v)).collect()
}

/// List scripts from manifest and last run per id from `data/script_runs.jsonl`.
#[utoipa::path(
    get,
    path = "/scripts",
    tag = "Scripts",
    responses(
        (status = 200, description = "Manifest + last runs", body = ScriptsListResponse),
        (status = 500, description = "Manifest unreadable")
    )
)]
pub async fn list_scripts(State(state): State<AppState>) -> ApiResult<Json<ScriptsListResponse>> {
    let repo = resolve_repo_root(&state);
    let repo_s = repo.display().to_string();
    let manifest_path = repo.join("tools").join("scripts-manifest.json");
    let manifest_path_s = manifest_path.display().to_string();
    let runs_path = repo.join("data").join("script_runs.jsonl");
    let runs_path_s = runs_path.display().to_string();

    let manifest_missing = !manifest_path.exists();
    let runs_missing = !runs_path.exists();

    let runner_configured = state
        .config
        .script_runner_url
        .as_ref()
        .is_some_and(|u| !u.trim().is_empty())
        && state
            .config
            .script_runner_token
            .as_ref()
            .is_some_and(|t| !t.trim().is_empty());

    let last = index_last_runs(&runs_path);

    let scripts: Vec<ScriptCatalogItem> = if manifest_missing {
        catalog_disk_only(&repo)
            .into_iter()
            .map(|s| manifest_script_to_item(s, &last, true))
            .collect()
    } else {
        let mf = read_manifest(&repo).map_err(ApiError::internal)?;
        merge_manifest_with_disk(&repo, mf)
            .into_iter()
            .map(|(s, auto)| manifest_script_to_item(s, &last, auto))
            .collect()
    };

    Ok(Json(ScriptsListResponse {
        repo_root: repo_s,
        manifest_path: manifest_path_s,
        manifest_missing,
        script_runs_path: runs_path_s,
        script_runs_missing: runs_missing,
        runner_configured,
        scripts,
    }))
}

/// Proxy a run request to `SCRIPT_RUNNER_URL` (localhost runner).
#[utoipa::path(
    post,
    path = "/scripts/{id}/run",
    tag = "Scripts",
    params(
        ("id" = String, Path, description = "Manifest script id (filename stem)")
    ),
    request_body = Option<RunScriptRequest>,
    responses(
        (status = 200, description = "Run finished", body = ScriptRunRecord),
        (status = 400, description = "Not runnable or unknown id"),
        (status = 503, description = "Runner not configured"),
        (status = 502, description = "Runner error")
    )
)]
pub async fn run_script(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<RunScriptRequest>>,
) -> ApiResult<Json<ScriptRunRecord>> {
    let derived = std::env::var("CLMM_SCRIPT_RUNNER_PORT").ok().and_then(|p| {
        let t = p.trim();
        if t.is_empty() {
            None
        } else {
            Some(format!("http://127.0.0.1:{t}"))
        }
    });
    let base = derived
        .as_ref()
        .map(|s| s.as_str())
        .or_else(|| {
            state
                .config
                .script_runner_url
                .as_ref()
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.as_str())
        })
        .ok_or_else(|| {
            ApiError::service_unavailable(
                "SCRIPT_RUNNER_URL is not set (start tools/script_runner/Start-ClmmScriptRunner.ps1)",
            )
        })?;
    let token = state
        .config
        .script_runner_token
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            ApiError::service_unavailable(
                "SCRIPT_RUNNER_TOKEN is not set (must match CLMM_SCRIPT_RUNNER_TOKEN on runner)",
            )
        })?;

    let repo = resolve_repo_root(&state);
    let manifest_path = repo.join("tools").join("scripts-manifest.json");
    let mf = if manifest_path.exists() {
        read_manifest(&repo).map_err(ApiError::internal)?
    } else {
        ManifestFile {
            schema_version: None,
            scripts: vec![],
        }
    };
    let entry = resolve_script_for_run(&repo, &mf, &id).ok_or_else(|| {
        ApiError::not_found(format!(
            "unknown script id: {id} (expected manifest entry or tools/{id}.ps1)"
        ))
    })?;
    if !entry.runnable {
        return Err(ApiError::bad_request(
            "script is marked not runnable (helper/example); use copy_command only",
        ));
    }

    let triggered = body
        .and_then(|b| b.triggered_by.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "api".to_string());

    let url = format!("{}/run", base.trim_end_matches('/'));
    let resp = HTTP
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        // Prefer query params to avoid HttpListener InputStream/headers edge cases.
        .query(&[
            ("script_id", id.as_str()),
            ("triggered_by", triggered.as_str()),
            ("token", token.as_str()),
        ])
        // HttpListener expects Content-Length even for an empty POST.
        .header(header::CONTENT_LENGTH, "0")
        .body(Vec::<u8>::new())
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("script runner request: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| ApiError::internal(format!("script runner body: {e}")))?;

    if !status.is_success() {
        let err_msg = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("error").and_then(|x| x.as_str()).map(str::to_string))
            .unwrap_or_else(|| text.clone());
        return Err(ApiError::bad_gateway(format!(
            "runner HTTP {status}: {err_msg}"
        )));
    }

    let record: ScriptRunRecord = serde_json::from_str(&text)
        .map_err(|e| ApiError::internal(format!("runner JSON: {e}: {text}")))?;

    Ok(Json(record))
}
