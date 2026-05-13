# AGENTS.md

## Cursor Cloud specific instructions

### Project overview

Bociarz LP — a Rust (edition 2024) monorepo with 8 crates and a React/TypeScript web dashboard. See `README.md` and `STARTUP.md` for full details.

**AI / „agent” in this repo:** see `doc/AI_AGENT_LAYER.md` (`AgentDecision` + apply-optimize, Position Agent, live `DecisionEngine`, audit JSONL — not a single generic chatbot).  
**Decision / orchestration layer (vision, shadow sims, phases):** `doc/DECISION_LAYER.md`.

### Required system dependencies

Installed once by the VM snapshot (not in the update script):

- `libssl-dev`, `libpq-dev`, `pkg-config` — needed for Rust native compilation (OpenSSL, PostgreSQL client)
- PostgreSQL 16 (local install, not Docker) — service must be running before the API server starts
- Rust stable (1.90+, via `rustup`), Node.js 18+ (pre-installed)

### PostgreSQL setup

A local PostgreSQL instance is used. The database `clmm_lp` with user `clmm_user` / password `clmm_password` is created during initial setup. To start PostgreSQL if not running:

```bash
sudo service postgresql start
```

### Environment file

`.env` is copied from `.env.example` during initial setup. It contains database URL, API config, and placeholder API keys. The API server reads `DATABASE_URL`, `API_PORT`, etc. from this file.

### Common commands

Standard build/test/lint commands are in the `Makefile`:

| Task | Command |
|------|---------|
| Build | `make build` or `cargo build --workspace` |
| Test | `make test` or `LOGLEVEL=WARN cargo test` |
| Lint (strict) | `make lint` (uses `-D warnings`; has pre-existing warnings) |
| Format | `make fmt` |
| Pre-push | `make pre-push` |

### Starting services

**Recommended (one terminal, API + dashboard):** `cd web && npm install && npm start` — same as `npm run dev:stack`; starts `clmm-lp-api` (port **8080**) and Vite (port **3000**); frees ports **3000**/**8080** first. See `STARTUP.md`.

**Manual:** PostgreSQL if required by your setup → **API**: `RUST_LOG=info cargo run --bin clmm-lp-api` → **Web**: `cd web && npm run dev` (Vite only; use when API is already running elsewhere).

**Docker (optional):** `docker compose up --build` from repo root or `make docker-up` — see `doc/DOCKER.md`. In Compose, set **`API_UPSTREAM`** (e.g. `http://api:8080`) so the Vite container can reach the API by service name.

### Known gotchas

- **Vite proxy**: `web/vite.config.ts` proxies `/api` and `/ws` to **`API_UPSTREAM`** or, by default, **http://127.0.0.1:8080**. For a different API port or Docker Compose, set **`API_UPSTREAM`** (and match `API_PORT` on the server if needed).
- **`make lint` pre-existing warnings**: The codebase has pre-existing clippy warnings (unused variables in `crates/api/src/services/strategy_service.rs`, various lints in `crates/cli/src/main.rs`) that cause `make lint` to fail since it uses `-D warnings`. `cargo build --workspace` and `cargo test` both succeed.
- **Cargo.lock is gitignored**: Each fresh checkout needs `cargo build` to resolve and lock dependencies.
- **package-lock.json is gitignored**: Each fresh checkout needs `npm install` in the `web/` directory.
- **Web frontend TypeScript check**: Run `npx tsc --noEmit` in `web/` to verify TypeScript types.
