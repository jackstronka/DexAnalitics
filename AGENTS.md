# AGENTS.md

## Cursor Cloud specific instructions

### Project overview

CLMM Liquidity Provider Strategy Optimizer — a Rust (edition 2024) monorepo with 8 crates and a React/TypeScript web dashboard. See `README.md` and `STARTUP.md` for full details.

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

Order: PostgreSQL → API Server → Web Dashboard

1. **PostgreSQL**: `sudo service postgresql start`
2. **API Server**: `RUST_LOG=info cargo run --bin clmm-lp-api` (port 8080)
3. **Web Dashboard**: `cd web && npm run dev` (port 3000)

### Known gotchas

- **Vite proxy mismatch**: `web/vite.config.ts` proxies `/api` and `/ws` to port **8081**, but the API server defaults to port **8080**. If you need the dashboard to proxy to the API, either change `API_PORT=8081` when starting the API or update the vite config.
- **`make lint` pre-existing warnings**: The codebase has pre-existing clippy warnings (unused variables in `crates/api/src/services/strategy_service.rs`, various lints in `crates/cli/src/main.rs`) that cause `make lint` to fail since it uses `-D warnings`. `cargo build --workspace` and `cargo test` both succeed.
- **Cargo.lock is gitignored**: Each fresh checkout needs `cargo build` to resolve and lock dependencies.
- **package-lock.json is gitignored**: Each fresh checkout needs `npm install` in the `web/` directory.
- **Web frontend TypeScript check**: Run `npx tsc --noEmit` in `web/` to verify TypeScript types.
