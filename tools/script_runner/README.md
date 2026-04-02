# CLMM script runner (localhost)

PowerShell HTTP listener that resolves a script by **`script_id`**: first entry in [`tools/scripts-manifest.json`](../scripts-manifest.json), else a file **`tools/{script_id}.ps1`** if it exists (same rule as `GET /api/v1/scripts`). Results append to [`data/script_runs.jsonl`](../../data/script_runs.jsonl). If the manifest is missing, only the on-disk `tools/*.ps1` fallback applies.

## Environment

| Variable | Required | Description |
|----------|----------|-------------|
| `CLMM_SCRIPT_RUNNER_TOKEN` | Yes | Shared secret; API sends `Authorization: Bearer &lt;token&gt;`. |
| `CLMM_REPO_ROOT` | No | Repository root (default: current directory). |
| `CLMM_SCRIPT_RUNNER_PORT` | No | Listen port (default: `9847`). |

## Start (Windows, repo root)

```powershell
. .\tools\script_runner\Start-ClmmScriptRunner.ps1
```

Or:

```powershell
$env:CLMM_SCRIPT_RUNNER_TOKEN = 'your-long-random-secret'
$env:CLMM_REPO_ROOT = 'F:\path\to\CLMM-Liquidity-Provider'
pwsh -File .\tools\script_runner\Start-ClmmScriptRunner.ps1
```

## API

- `GET http://127.0.0.1:9847/health` — no auth; returns `{ "ok": true }`.
- `POST http://127.0.0.1:9847/run` — JSON body `{ "script_id": "quick_verify_data" }`, header `Authorization: Bearer &lt;token&gt;`.

The API server proxies runs when `SCRIPT_RUNNER_URL` (e.g. `http://127.0.0.1:9847`) and `SCRIPT_RUNNER_TOKEN` are set. See [`doc/UI_REQUIREMENTS_PHASE1.md`](../../doc/UI_REQUIREMENTS_PHASE1.md).

## JSONL row shape (`data/script_runs.jsonl`)

Each line is one JSON object:

- `schema_version` (number, use `1`)
- `script_id` (string)
- `ts_utc` (RFC3339)
- `ok` (bool)
- `exit_code` (int)
- `duration_ms` (uint)
- `stdout_excerpt`, `stderr_excerpt`, `error_excerpt` (strings, optional)
- `triggered_by` (string, e.g. `runner_api`)

Do not expose this listener to the public internet.
