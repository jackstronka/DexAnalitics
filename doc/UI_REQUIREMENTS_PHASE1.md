# UI — wymagania fazy 1 (początkowa)

Dokument zamraża zakres produktowy dla dashboardu operatora (bot CLMM / Orca + analiza strategii). Szczegóły techniczne ledgerów: [`POSITION_REGISTRY.md`](POSITION_REGISTRY.md), [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md).

## Środowisko frontendu (obowiązuje niezależnie od Docker)

| Wymaganie | Uwagi |
|-----------|--------|
| **Node.js** | 18+ (`web/`), `npm install` w `web/`. |
| **Backend** | Panel zakłada działające API pod proxy Vite (domyślnie `http://127.0.0.1:8080`). Zobacz [`STARTUP.md`](../STARTUP.md), baner „brak API” w UI. |
| **Docker (opcjonalnie)** | Pełny stack w Compose: [`DOCKER.md`](DOCKER.md). **Na Windows: Docker Desktop musi być uruchomiony** — inaczej błąd pipe `dockerDesktopLinuxEngine`. |
| **Zmienne dev** | Opcjonalnie `VITE_DEV_WALLET_PUBKEY` — [`web/.env.example`](../web/.env.example). |

## Status implementacji (faza 1) — skrót

| Sekcja | Stan | Gdzie w kodzie / uwagi |
|--------|------|-------------------------|
| **§1 Skrypty** | Zrealizowane (szczegóły → poniżej) | [`web/src/pages/Scripts.tsx`](../web/src/pages/Scripts.tsx), [`GET /api/v1/scripts`](../crates/api/src/handlers/scripts.rs), [`tools/script_runner/`](../tools/script_runner/README.md). |
| **§2 Portfel USD** | Zrealizowane | [`Wallet.tsx`](../web/src/pages/Wallet.tsx), `GET /analytics/portfolio`, ApiDataHint o niepełności danych. |
| **§3 Pozycje** | Częściowo | Lista monitora + szczegóły z API; on-chain skan: [`Positions.tsx`](../web/src/pages/Positions.tsx), `GET /orca/positions-by-owner`. `logical_position_id` — roadmap (§7). |
| **§4 Koszty / ledger** | Częściowo | [`PositionDetail.tsx`](../web/src/pages/PositionDetail.tsx) — zakładka ledger: `tx_fee_lamports`, sesje; IL ledger: `tx_cost_lamports`. Pełne `fee_payer_net_lamports_delta` w tabeli — do rozszycenia przy bogatszym API/JSONL. Szacunki IL z PnL oznaczone jako metryki z monitora, nie „księgowość”. |
| **§5 Akcje** | Częściowo | Collect / rebalance / decrease / close przez REST; **zamknięcie z `window.confirm`**. Flow **unsigned tx + Phantom** — tam gdzie API to eksponuje (roadmap / endpointy tx). |
| **§6 Runner** | Dokumentacja + API | [`tools/script_runner/README.md`](../tools/script_runner/README.md), env na API. |
| **§7 Roadmap** | Otwarte | Agregacja lifecycle, PL/EN — poza faza 1. |

**Zasada:** nowe widoki mają **spinać się** z tym dokumentem; odchylenia dopisywać tutaj albo w `ENGINEERING_NOTES.md` (`keywords:`).

## 1. Katalog skryptów (`tools/*.ps1`)

- Lista skryptów z repozytorium wraz z **krótkim opisem** (tooltip / kolumna pomocy).
- Dla każdego: **ostatnia data uruchomienia**, **status** (OK / błąd), przy błędzie **fragment komunikatu** (pełny stderr dostępny w szczegółach lub w pliku runów).
- **Akcje**: uruchomienie przez **runnera** (localhost, allowlist), **kopiowanie komendy** CLI, ewentualnie link do dokumentacji w repo.
- Źródło metadanych: [`tools/scripts-manifest.json`](../tools/scripts-manifest.json) — **API scala manifest z listą plików `tools/*.ps1`** (pierwszy poziom), żeby w UI widać było też skrypty jeszcze niedopisane do JSON (oznaczenie `auto_discovered`). Historia uruchomień: [`data/script_runs.jsonl`](../data/script_runs.jsonl) (runner). Lokalny runner: [`tools/script_runner/Start-ClmmScriptRunner.ps1`](../tools/script_runner/Start-ClmmScriptRunner.ps1) — to samo rozstrzyganie co API (manifest albo `tools/{id}.ps1`).

### §1 — mapowanie wymagań → implementacja (audyt)

| Wymaganie | Status | Gdzie / jak |
|-----------|--------|-------------|
| Krótki opis, pomoc dla operatora | ✅ | Kolumny **Summary**, **when_to_use** (z manifestu); atrybuty `title` (tooltip przeglądarki); **risk** obok id. |
| Ostatnia data uruchomienia | ✅ | Kolumna **Last run** (`last_run.ts_utc` z JSONL). |
| Status OK / błąd | ✅ | Kolumna **Status** z `last_run.ok`. |
| Fragment komunikatu błędu | ✅ | Kolumna **Error** (`error_excerpt`); szerszy podgląd **stdout/stderr** w modalnym **„Logi”** (dane z API / JSONL). |
| Uruchomienie przez runnera (allowlist) | ✅ | **Run** → `POST /api/v1/scripts/{id}/run` → runner; wymaga `SCRIPT_RUNNER_URL` + `SCRIPT_RUNNER_TOKEN`. |
| Kopiowanie komendy CLI | ✅ | **Copy** — `pwsh -NoProfile -File <path>`. |
| Dokumentacja w repo | ✅ (katalog) | Nagłówek strony: odsyłacz do [`doc/SCRIPTS_CATALOG.md`](SCRIPTS_CATALOG.md) (pełny spis); per-skrypt — opis w manifeście + plik `.ps1` w repo. |
| Korektny katalog repo na hoście API | ✅ | `CLMM_REPO_ROOT` lub autowykrycie root (spacer od `cwd` / `exe`); zob. `resolve_repo_root` w API. |

**Poza UI (operacyjnie):** plik runów `data/script_runs.jsonl` można przeglądać lokalnie — to „źródło prawdy” dla pełnych logów, jeśli excerpt w API jest przycięty.

## 2. Portfel (USD)

- Widok **łącznej wartości** i podsumowania PnL zgodnie z API (`GET /analytics/portfolio`) oraz listą pozycji z `value_usd`.
- Jawna niepełność: wartości zależą od RPC i oznaczeń cen — nie obiecujemy księgowości co do centa (por. reguły danych w repo).

## 3. Pozycje LP

- **Otwarte**: lista + szczegóły (ticki, range, in-range, wartość, PnL z API).
- **Historia / rebalanse**: operacje zapisane w ledgerze JSONL — filtrowanie po adresie pozycji; grupowanie po `rebalance_session_id` tam, gdzie występuje (łączenie swapów i kroków bota w jednej sesji).
- Rozróżnienie „nowy NFT pozycji” vs „ten sam łańcuch strategii”: on-chain NFT może się zmieniać przy close+open; **linia biznesowa** powinna być korelowana sesją (`rebalance_session_id`) i/lub przyszłym `logical_position_id` (do decyzji domain — roadmap w notatkach inżynierskich).

## 4. Koszty / ledger (P&amp;L)

- Wyświetlanie **zmierzonych** opłat sieci i delt płatnika z wierszy ledgera (`tx_fee_lamports`, `fee_payer_net_lamports_delta`, zdarzenia `orca_bot` / `cli`).
- Osobno: szacunki IL / fee ze snapshotów lub backtestu — **etykieta „szacunek”**, nie mylić ze zdarzeniami już zaksięgowanymi on-chain.

## 5. Akcje na pozycjach (zgodnie z API)

- Zbieranie opłat, rebalance, zmniejszenie płynności, zamknięcie; flow **unsigned tx + podpis** tam, gdzie API to eksponuje.
- Akcje destruktywne z potwierdzeniem.

## 6. Runner skryptów

- Proces na maszynie operatora (Windows: PowerShell), nasłuch localhost, token, **allowlist** ścieżek w `tools/`.
- API proxy: `SCRIPT_RUNNER_URL`, `SCRIPT_RUNNER_TOKEN`; katalog repo: `CLMM_REPO_ROOT`.
- Uruchomienie: [`tools/script_runner/README.md`](../tools/script_runner/README.md).

## 7. Roadmap (poza fazą 1)

- Endpoint agregujący lifecycle pozycji (zamiast składania tysięcy wierszy w przeglądarce).
- Pełna spójność językowa UI (PL/EN).
