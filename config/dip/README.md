# Dip Agent config (Dex Analytics)

Tracked seed for the Jupiter Buy-the-Dip leg. Docs: [`doc/dip-agent/README.md`](../../doc/dip-agent/README.md).

| File | Role |
|------|------|
| `runtime.yaml` | `mode: propose-only`, size limits, monitor knobs |
| `mints.yaml` | SOL/USDC mints + zones A/B/C (human source) |
| `zones.json` | Same SOL A/B/C bands for Rust/API (`dip-snapshot`, `GET /dip/snapshot`) |
| `universe.yaml` | Monitor universe seed |
| `context/` | Seed WORLD / portfolio / watchlist / news / SOL journal |

Working copy (gitignored under `/data/`): copy `context/` → `data/dip/context/` before mutating locally.
