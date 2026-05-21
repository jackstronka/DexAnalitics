# BUGS Registry

Purpose: durable, searchable bug memory for AI/humans across sessions/models.

Rules for entries:
- Keep newest entries at the top.
- Use stable ids: `BUG-YYYYMMDD-XX`.
- Include `keywords:` for grep/AI recall.
- Record both: what was wrong and how it was fixed.
- Mark status: `open`, `fixed`, `regressed`, `wontfix`.

## Entry Template

### BUG-YYYYMMDD-XX — Short title

status: open|fixed|regressed|wontfix  
severity: low|medium|high|critical  
reported_by: user|ai|monitoring  
first_seen: YYYY-MM-DD  
fixed_in: <commit-sha-or-branch-or-empty>  
keywords: comma,separated,tokens,for,search

- **Symptom:** user-visible behavior.
- **Root cause:** technical reason.
- **Fix:** what changed.
- **Guards/tests:** checks to prevent recurrence.
- **Paths:** `file/a`, `file/b`

---

### BUG-20260521-06 — Chain net PnL −100% when current $0 on closed position

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-21  
fixed_in: local  
keywords: net_pnl_usd, current_value_usd, closed position, -1.000%, chain_headline_end_nav, lineage_node_end_nav, live snapshots

- **Symptom:** Wynik ekonomiczny łańcucha: baseline ~$9.90, **current $0.000**, net PnL **−$9.905 (−1.000%)** mimo zebranych LP fees (~$0.03) i opłat tx ~$0.0035.
- **Root cause:** Po zamknięciu PDA `current_value_usd` w totals zostawało 0 (brak on-chain NAV); wzór dawał net PnL ≈ −baseline. `reconcile_stream_pnl_totals_with_nodes` nie traktowało „current=0 przy znanym baseline” jako do naprawy.
- **Fix:** `lineage_node_end_nav_usd` / `chain_headline_end_nav_usd` — end z `chain_history_end_value_usd` lub estymata zamknięcia (baseline + LP fees + cashflow − tx); `refresh_lineage_totals_from_nodes` podnosi current i przelicza net PnL.
- **Guards/tests:** `chain_headline_end_nav_uses_close_estimate_when_current_zero`, `refresh_lineage_totals_repairs_zero_current_closed_chain_net_pnl`.
- **Paths:** `position_stream_lineage.rs`

---

### BUG-20260521-05 — Chain-history Postgres: baseline/HODL $0, net PnL = current (stale totals_json)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-21  
fixed_in: local  
keywords: chain-history, totals_json, baseline_value_usd, hodl_value_usd, net_pnl_usd, live snapshots, HySRBC91, refresh_lineage_totals_from_nodes, materialize

- **Symptom:** Zakładka Historia (Postgres), `source: live snapshots` — baseline $0, HODL $0, net PnL ≈ current (~$9.95, +0%); IL vs HODL wszystko $0 mimo 3 PDA w łańcuchu i zebranych fee (~$0.05). `GET stream-lineage` dla tego samego anchor pokazywał poprawne sumy (~$9.97 baseline).
- **Root cause:** `GET …/chain-history` zwracał zamrożone `totals_json` z wczesnej materializacji (przed snapshotami / pełnym łańcuchem). Węzły w tabeli miały poprawne `start_value_usd`, ale nagłówek totals nie był przeliczany przy odczycie.
- **Fix:** `refresh_lineage_totals_from_nodes` na odczycie chain-history: odbudowa z węzłów + `reconcile_stream_pnl_totals_with_nodes`; rozszerzony warunek placeholder (`baseline=0`, `current>0`, `node_fallback_unavailable`). **Follow-up:** `refresh_chain_history_node_fees_from_ledger` — fallback `tx_fee_lamports` z lifecycle JSONL gdy PSLR ma 0. **Follow-up 2 (tx fees USD $0):** drugi pass `apply_tx_fees_usd_from_lamports_on_nodes` gdy λ>0 a USD=0 (nagłówek był uzupełniany bez węzłów); `sol_usd_for_tx_fees` najpierw event spot z Postgres; stream-lineage też woła refresh opłat + `refresh_lineage_totals_from_nodes`.
- **Guards/tests:** `refresh_lineage_totals_repairs_stale_chain_history_meta_baseline_zero` (`cargo test -p clmm-lp-api refresh_lineage_totals`).
- **Paths:** `position_chain_history.rs`, `position_stream_lineage.rs`

---

### BUG-20260521-04 — Sesja teraz ~−$20 przy open ~$10 (podwójny SESSION GL)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-21  
fixed_in: local  
keywords: session-balances, SessionBalancesPanel, current_value_usd, Sesja teraz, gl_pslr_match, open_position, journal, lifecycle, amount_a_cap, open_amount_raw, 4FQfsB9a, 9dc9a854, operator_api

- **Symptom:** Po open ~$9.90 panel „Punkt odniesienia” pokazywał **Sesja teraz (USD, ceny z open)** ≈ **−$19.95** (~2× wdrożonego kapitału).
- **Root cause:** `POST /positions` (open) zapisywał do SESSION GL **capy** z requestu (`amount_a/b`), a ingest lifecycle — **`open_amount_*_raw`** on-chain; różne `event_id` → oba wpisy. Metryki liczyły zmaterializowane GL.
- **Fix:** Journal nie duplikuje principal dla `open_position`/`close_position` gdy włączone lifecycle posting; `GET session-balances` i metryki używają **PSLR**, gdy `!gl_pslr_match` (`gl_session_shadow_pslr_corrected`).
- **Guards/tests:** `journal_open_close_deferred_when_lifecycle_posting_on`, `open_lifecycle_single_post_matches_cap_plus_onchain_double`, `session_balances_for_metrics_prefers_pslr_when_gl_doubles_open` (`cargo test -p clmm-lp-data wallet_session`, `cargo test -p clmm-lp-api wallet_gl_posting`).
- **Paths:** `wallet_gl_posting.rs`, `wallet_session.rs`, `handlers/wallets.rs`, `SessionBalancesPanel.tsx`

---

### BUG-20260521-03 — Bulk send-first close: 6018 po confirm mimo „wysłane”

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-21  
fixed_in: local  
keywords: close-all, send-first, 6018, slippage, bulk-close, TokenMinSubceeded

- **Symptom:** Batch `1/2 potwierdzone · 1 błąd` — pozycja z sygnaturą tx, błąd `close confirm: … Custom(6018)`; pozycja nadal otwarta on-chain.
- **Root cause:** Send-first traktował broadcast jako sukces (bez symulacji); domyślne **100 bps** close slippage + brak retry w ścieżce bulk; 6018 ujawniał się dopiero w `finalize_bulk_close_after_confirm`.
- **Fix:** Bulk default **200 bps** (`options.slippage_bps`, UI); poll po submit + jeden retry przy 6018 (≥500 bps); finalize też jeden retry submit; hint UI „pozycja nadal otwarta”.
- **Guards/tests:** `position_close_ops::tests` (`resolve_bulk_close_slippage`, `bump_close_slippage_for_6018_retry`, `is_close_slippage_6018`).
- **Paths:** `position_close_ops.rs`, `position_close_all.rs`, `position_service.rs`, `rebalance.rs`, `Positions.tsx`

### BUG-20260521-02 — Close-all preview: pełny skan monitora przy 2 zaznaczonych PDA

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-05-21  
fixed_in: local  
keywords: close-all-preview, explicit-scope, collect_monitored, positions-ui, slow-load, PERF-PR1

- **Symptom:** Po dodaniu zamknięcia zbiorczego panel confirm / lista pozycji wydają się bardzo wolne; preview close-all timeout do 60 s.
- **Root cause:** `resolve_close_all_addresses` zawsze wołało `collect_monitored_position_addresses` (ten sam koszt RPC co `GET /positions`) nawet przy `scope=explicit` i kilku adresach.
- **Fix:** `explicit` → tylko `req.addresses`; debounce preview 300 ms; mniej refetch strategii; batch poll 8 s + visibility pause.
- **Guards/tests:** `explicit_scope_uses_request_addresses_without_monitored_union` (`cargo test -p clmm-lp-api position_close_all::tests`).
- **Paths:** `crates/api/src/services/position_close_all.rs`, `web/src/pages/Positions.tsx`

---

### BUG-20260521-01 — Close-all banner: „2 w toku” bez widocznego postępu

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-05-21  
fixed_in: local  
keywords: close-all, bulk-close, positions-ui, pending, slow-rpc, batch-polling, UX

- **Symptom:** Po starcie zamknięcia wybranych pozycji banner pokazuje `0/2 zamknięte · 2 w toku` przez długi czas; operator ma wrażenie, że „nic się nie dzieje”.
- **Root cause:** Worker zamyka **sekwencyjnie** z synchronicznym `send+confirm` (collect fees + close ≈ 2× do 90 s na tx). Status batcha (`items[]`) aktualizuje się dopiero po **zakończeniu** danej pozycji; UI pokazywało tylko agregat summary, bez listy pozycji, czasu trwania ani komunikatu o wolnym RPC. Po restarcie API batch w pamięci znika → polling 404 bez jawnego komunikatu.
- **Fix:** Banner: lista `items` ze statusem per PDA, elapsed timer, hint o 2–5 min/pozycję; błąd gdy batch not found. Log `close-all: closing position` w workerze.
- **Guards/tests:** Ręcznie: 2 pozycje → widać `on-chain (collect + close)…` i rosnący elapsed; po zakończeniu `zamknięte` / błąd z `error`.
- **Paths:** `web/src/pages/Positions.tsx`, `web/src/lib/i18n.tsx`, `crates/api/src/services/position_close_all.rs`

---

### BUG-20260520-03 — Backfill SESSION GL: `wartość zbyt długa dla typu znakowego zmiennego (64)`

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-20  
fixed_in: local  
keywords: wallet_gl_posting, session-balances, backfill, event_id, VARCHAR(64), lifecycle, signature, migrate 012

- **Symptom:** `POST /wallets/session-balances/backfill` → `Internal error: session backfill failed: … wartość zbyt długa dla typu znakowego zmiennego (64)` przy „Zapisz lifecycle do księgi GL”.
- **Root cause:** `wallet_gl_posting.event_id` i `wallet_gl_balance.last_event_id` były `VARCHAR(64)`; idempotentny klucz to `lifecycle:{signature}` (~98 znaków dla base58 Solana).
- **Fix:** Migracja `012_wallet_gl_event_id_widen.sql` → `VARCHAR(128)`; test długości `lifecycle_posting_event_id_fits_wallet_gl_column`.
- **Guards/tests:** `cargo test -p clmm-lp-data lifecycle_posting_event_id_fits_wallet_gl_column`; po restarcie API (migrate) ponowić backfill.
- **Paths:** `crates/data/migrations/012_wallet_gl_event_id_widen.sql`, `crates/data/src/repositories/database.rs`

---

### BUG-20260520-02 — Kapitał sesji: ~$97 „przed open” przy portfelu &lt; $80 (SOL raw zaksięgowany jako USDC)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-20  
fixed_in: local  
keywords: wallet_session, session-balances, pre_open_value_usd, close_amount_a_raw, token_mint_a, pool_address, fee_payer_token_deltas, 5hb7waSR, 72a3fbc5, SESSION GL, phantom USDC

- **Symptom:** Panel „Kapitał sesji rebalance” pokazywał „Sesja tuż przed open” ~$97 i Δ ~−$8 vs portfel on-chain ~$35–40 (kilka USDC + ~0.3 SOL + LP ~$8). Wcześniejsza analiza AI uznała liczby za spójne — bez weryfikacji RPC.
- **Root cause:** Wiersz `bot_close_position` bez `details.token_mint_a/b`; `pool_mints_from_lifecycle` brał kolejność kluczy z `fee_payer_token_deltas` (USDC pierwszy) i księgował `close_amount_a_raw` (lamports SOL, np. 94 165 873) na mint USDC → ~94 USDC fantom + swap ≈ $97. Open/close w executorze nie zapisywały mintów puli w `details`.
- **Fix:** `wallet_session`: mapa curated `pool_address` → minty; **brak** fallbacku z `fee_payer_token_deltas` dla close/open/collect principal; `metrics_trusted` + `mint_resolution` w API; ostrzeżenie w `SessionBalancesPanel`. `enrich_open_close_ledger_details` dopisuje `token_mint_a/b` z odczytu puli (open i close).
- **Guards/tests:** `close_without_details_mints_uses_pool_address_not_fee_payer_deltas` (`cargo test -p clmm-lp-data wallet_session`); regresja: porównać `GET /wallets/session-balances?session_id=72a3fbc5-…` z `effective-balances` — pre_open USD &lt; ~20 dla tego close.
- **Paths:** `crates/data/src/wallet_session.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/api/src/models.rs`, `crates/api/src/services/wallet_gl_posting.rs`, `web/src/components/SessionBalancesPanel.tsx`

---

### BUG-20260520-01 — Stream-lineage totals: zły HODL/cashflow/PnL po operator_api open (~$10 → HODL ~$4.86)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-20  
fixed_in: local  
keywords: stream-lineage, wartość start, hodl_value_usd, realized_cashflow_usd, net_pnl_usd, operator_api, open_quote_estimated_value_usd, baseline_open, fee_payer_token_deltas, position_stream_pnl, 7UhNx5sqobK6Cefc9bhLNzpas5wJyRCH1iMBhHJt2TGu

- **Symptom:** Dla PDA otwartego z experiment/operator API (~$10 SOL/USDC) kolumna węzła **wartość start** ≈ $10 była OK, ale sumy stream-lineage: `hodl_value_usd` ≈ $4.86 (tylko USDC), `realized_cashflow_usd` ≈ −$4.86, `net_pnl_usd` ≈ −$4.86, `current_value_usd` utknął na baseline.
- **Root cause:** (1) Brak `open_quote_estimated_value_usd` / `open_target_usd` w lifecycle operator open. (2) Baseline snapshot z samych `fee_payer_token_deltas` pomijał nogę SOL. (3) `stream_pnl` cashflow sumował delty open/close jako realized cashflow. (4) Totals liczone przed persist snapshotów; brak live `current` gdy jedyny wiersz to `baseline_open`.
- **Fix:** `enrich_open_close_ledger_details` zapisuje quote USD z on-chain `open_amount_*_raw`; baseline preferuje `open_amount_raw` / quote caps; cashflow pomija open/close principal; lineage await persist przed totals + self-seed `live_current`; **totals liczone po węzłach** z `reconcile_stream_pnl_totals_with_nodes` gdy HODL/current z DB odbiegają od wierszy tabeli.
- **Guards/tests:** `cashflow_skips_open_and_close_principal_events`, `current_snapshot_stale_when_baseline_open_or_same_ts` (`cargo test -p clmm-lp-api position_stream_pnl`); `baseline_open_prefers_open_amount_raw_over_fee_payer_deltas`, `reconcile_stream_pnl_totals_with_nodes_repairs_degraded_hodl_and_stale_current` (`stream_lineage`); `insert_open_quote_usd_fields_from_onchain_amounts` (`cargo test -p clmm-lp-execution insert_open_quote`).
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`, `crates/api/src/services/position_stream_pnl.rs`, `crates/execution/src/strategy/rebalance.rs`

---

### BUG-20260514-06 — Logs/rebalances: „Fees zebrane” bez nogi SOL (tylko USDC / brak rozbicia) przy długim łańcuchu

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-05-14  
fixed_in: local  
keywords: stream-lineage, fast_long_chain_metrics, Fees zebrane, fees_collected_token_a_ui, pool_mint_a, position_stream_valuation_snapshots, lp_fees_collected_usd_from_ledger_db_batch, logs-rebalances

- **Symptom:** W **Logs / rebalances** kolumna **Fees zebrane** pokazywała sumę USD lub tylko jedną nogę (np. USDC), bez widocznej drugiej nogi (SOL), mimo że Solscan pokazuje obie nogi.
- **Root cause:** Ścieżka **`node_metrics_fast_for_chain`** brała `token_mint_a/b` tylko ze snapshotów waluacji; dla wielu PDA w środku łańcucha minty były **puste**, podczas gdy rollup fee z ledgera (`by_mint_ui`) miał poprawne klucze mintów z **puli** — UI nie mógł zmapować kwot na etykiety SOL/USDC.
- **Fix:** `lp_fees_collected_usd_from_ledger_db_batch` zapisuje **`pool_mint_a` / `pool_mint_b`** (kolejność Whirlpool). Przed złożeniem węzła fast-path i przy **`refresh_chain_history_node_fees_from_ledger`**: **`fill_missing_lineage_mints_from_fee_metric`** + **`fees_collected_token_ui_for_fee_metric`** uzupełniają minty i `fees_collected_token_*_ui`.
- **Guards/tests:** `fill_missing_mints_from_fee_metric_restores_sol_usdc_fee_legs` (`cargo test -p clmm-lp-api stream_lineage`).
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`

---

### BUG-20260514-05 — GET chain-history: 404 „no materialized…” mimo że snapshot w Postgres jest (inny PDA w URL)

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-05-14  
fixed_in: local  
keywords: chain-history, position_chain_history_meta, chain_anchor_pubkey, position_pubkey, rotation, 404, load_chain_history_from_db

- **Symptom:** Zakładka **Historia (Postgres)** — komunikat jak przy braku materializacji (`Not found: no materialized chain-history…`), choć wcześniej dane były widoczne / snapshot istnieje w DB.
- **Root cause:** (1) Odczyt szukał wyłącznie **`chain_anchor_pubkey =` URL** — materializacja pod **innym** PDA w tej samej rotacji → 404. (2) **Ogon łańcucha** (`stream-lineage` ma więcej PDAs niż zapisane `chain_json` / `nodes` w momencie materializacji): bieżący człon **nie występuje** w Postgresie → same SQL-heurystyki nadal 404.
- **Fix:** `load_chain_history_from_db`: kotwica przez `nodes` / `meta.entry` / `chain_json @> jsonb_build_array(pda)`; jeśli brak — **spacer** po łańcuchu z `resolve_lineage_chain_for_stream_pnl` (jak stream-lineage) do pierwszego PDA z materializacją; odczyt pod `effective_anchor`; w `note` dopisek przy remapie.
- **Guards/tests:** `cargo test -p clmm-lp-api position_chain_history`; regresja manualna: GET z członka ≠ anchor przy istniejącej materializacji.
- **Paths:** `crates/api/src/services/position_chain_history.rs`

### BUG-20260514-03 — Po migracji 009 API bez Postgres (503 chain-history), wcześniej działało

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-14  
fixed_in: local  
keywords: migrate, Database::migrate, semicolon split, wallet_gl_token_account, 009_wallet_gl_curated_tokens_and_pools, DATABASE_URL, connect_db_best_effort, chain-history

- **Symptom:** Po ostatnich zmianach w repo **503** *Postgres is not connected* na chain-history; `/health` OK; użytkownik nic nie zmieniał w `.env`.
- **Root cause:** `009_wallet_gl_curated_tokens_and_pools.sql` zawierał w stringu SQL **`notes`** średnik (`…token order); curated…`). `Database::migrate` dzieli pliki po **`;`** (po usunięciu linii `--`), więc statement się **rozcinał** na niepoprawne fragmenty → **błąd migracji** → `connect_db_best_effort` zwracał **`db: None`** dla całego API.
- **Fix:** Usunięto średnik z treści `notes` (zamiana na przecineek). Osobno: `Start-ClmmApi-8081.ps1` przekazuje `DATABASE_URL` (inna klasa problemów na Windows).
- **Guards/tests:** unikać `;` w literałach stringów w plikach migracji dopóki runner jest naiwny; rozważyć parser SQL lub migracje jednoplikowe bez `;` w stringach.
- **Paths:** `crates/data/migrations/009_wallet_gl_curated_tokens_and_pools.sql`, `crates/data/src/repositories/database.rs`

### BUG-20260514-02 — Postgres chain-history: zły start (~open_quote), brak „end” mimo close, brak reopen w tabeli, zera fee/tx w UI

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-05-14  
fixed_in: local  
keywords: chain-history, chain_history_start_value_usd, chain_history_end_value_usd, enrich_chain_history_nodes_open_quote_baseline_lift, baseline_value_usd, current_value_usd, position_stream_edges, reopen, lifecycle JSONL, resolve_lineage_chain_for_stream_pnl, refresh_chain_history_node_fees_from_ledger, position_stream_ledger_rows, rollup_lineage_chain_costs

- **Symptom:** Zakładka **Historia (Postgres)** — zły **start** (~open_quote), **„—”** przy **end** mimo close, **reopen** nie w tabeli; ponadto zera **tx fee** / **fees collected** mimo danych w ledgerze.
- **Root cause:** (1) Łańcuch z DB urwał się przed reopen, bo lifecycle fallback działał tylko przy `chain.len() <= 1`. (2) Pętla po enrich nadpisywała **`chain_history_start_value_usd`** zawsze z **`baseline_value_usd`** — gdy baseline zostawał przy open_quote, **nadpisywała lepszy `start_value_usd` z kolumny SQL** (regresja). (3) `GET chain-history` nie odświeżał fee z **`position_stream_ledger_rows`** — UI brało głównie **`raw_snapshot`** z materializacji (często zera zanim ledger się zapełnił). (4) `chain_history_end_value_usd` — backfill tylko gdy pole JSON puste, a `current` po enrich &gt; 0.
- **Fix:** Prefiksowe przedłużenie łańcucha lifecycle w `resolve_lineage_chain_for_stream_pnl` (testy `prefer_lifecycle_lineage_if_extends_db_prefix_*`). W `load_chain_history_from_db`: backfill `chain_history_*` **tylko** gdy pole JSON było puste; **`refresh_chain_history_node_fees_from_ledger`** + przeliczenie `net_pnl_*`; suma kosztów z **`rollup_lineage_chain_costs`** po odświeżeniu (fallback na meta JSON). **Read:** merge `chain_json` z live `resolve_lineage_*` gdy meta to sam ogon (`[new]`) lub krótszy prefiks; brakujące PDA z **`node_metrics`**.
- **Guards/tests:** `cargo test -p clmm-lp-api prefer_lifecycle_lineage_if_extends_db_prefix_*`; `cargo build -p clmm-lp-api`
- **Paths:** `crates/api/src/services/position_chain_history.rs`, `crates/api/src/services/position_stream_lineage.rs`

### BUG-20260514-01 — Zakładka Postgres chain-history: pusta „wartość start”, stream-lineage pokazuje kwotę

status: fixed  
severity: low  
reported_by: user  
first_seen: 2026-05-14  
fixed_in: local  
keywords: chain-history, stream-lineage, baseline_value_usd, raw_snapshot, position_stream_ledger_rows, open_quote_baseline_lift, UI start value

- **Symptom:** W jednej zakładce lineage (stream) widać sensowną **wartość start** / baseline, w **Historia (Postgres)** ta sama kolumna pusta (`—`).
- **Root cause:** `GET …/chain-history` zwracał węzły **wyłącznie** z zamrożonego `raw_snapshot` z momentu materializacji. Live `GET …/stream-lineage` po każdym żądaniu ponownie stosuje lift z **aktualnej** tabeli `position_stream_ledger_rows` (+ lifecycle), więc po dopisaniu open do DB stream ma baseline, a snapshot PG — nie, dopóki nie zrobi się pełnego refresh materialize. Frontendowy fallback z JSONL ma ten sam problem co stream (ogon pliku vs pełna historia w DB).
- **Fix:** (1) Lift open-quote przy odczycie. (2) Overlay z kolumn SQL na zdeserializowany węzeł + przeliczenie `net_pnl_*`. (3) **10x UI/API:** jawne pola `chain_history_start_value_usd` / `end` / `current` w JSON — zakładka Postgres rysuje te liczby wprost, zamiast tylko heurystyk lineage.
- **Guards/tests:** `cargo check -p clmm-lp-api`; regresja manualna: GET chain-history vs stream przy starym snapshotcie.
- **Paths:** `crates/api/src/services/position_chain_history.rs`, `crates/api/src/services/position_stream_lineage.rs`

---

### BUG-20260513-04 — Stream lineage (długi łańcuch): wiersze z samymi „—” (brak dat / baseline / mintów w UI)

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-05-13  
fixed_in: local  
keywords: stream-lineage, node_metrics_fast_for_chain, position_stream_valuation_snapshots, lifecycle JSONL, opened_ts_utc, closed_ts_utc, token_mint_a, open_quote_estimated_value_usd, UI dashes

- **Symptom:** Przy łańcuchu **>8** PDA (np. 16–17 NFT) tabela lineage pokazywała **„—”** dla `opened` / `closed`, pustych baseline/current oraz komunikat „Brak sumy USD w API…” mimo że `orca_position_lifecycle.jsonl` zawiera pełne `bot_open` / `bot_close` i `open_quote_*`.
- **Root cause:** `node_metrics_fast_for_chain` bierze `opened_ts_utc` / `closed_ts_utc` **wyłącznie** z tabeli `position_stream_valuation_snapshots`; dla wielu środkowych PDA snapshotów brak → API zwracało puste pola. Mapa `open_quote` dla `apply_open_quote_baseline_lift` pochodziła tylko z `position_stream_ledger_rows` (ingest) — gdy DB nie nadążała za JSONL, lift nie miał danych.
- **Fix:** Po zbudowaniu węzłów: `hydrate_lineage_open_close_ts_and_mints_from_lifecycle` (pierwszy open / ostatni close + minty z lifecycle); przed liftem: `merge_open_quote_usd_from_lifecycle_rows` łączy open-quote z JSONL z mapą z DB (max per PDA).
- **Guards/tests:** `merge_open_quote_usd_from_lifecycle_rows_fills_missing_db_map`, `hydrate_lineage_fills_open_close_ts_and_mints_from_lifecycle` (`cargo test -p clmm-lp-api <filter>`).
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`, `doc/ENGINEERING_NOTES.md`

---

### BUG-20260513-03 — Stream lineage: zaniżony start ($0.84) vs koniec ($3.30) na długim łańcuchu (fast path)

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-05-13  
fixed_in: local  
keywords: stream-lineage, baseline_value_usd, current_value_usd, node_metrics_fast_for_chain, open_quote_estimated_value_usd, rotation, end_close snapshot

- **Symptom follow-up (2026-05-13):** Ten sam zły start (~$0.84 / ~$3.30) na **krótkich** łańcuchach (≤8 PDA) — pierwsza naprawa ograniczała `apply_open_quote_baseline_lift_after_lineage_fallbacks` do `chain.len() > 8`; otwarty wiersz pokazywał **current** tylko ze snapshotu DB (np. „—”) zamiast ~$8.7 z live.
- **Root cause:** Przy `chain.len() > 8` `node_metrics_fast_for_chain` podnosił baseline z open-quote tylko gdy `current_value_usd > 0` i baseline &lt; 60% current. Bez snapshotu `end_close` w DB **`current_value_usd` było 0** do czasu `apply_end_value_fallback_from_next_baseline` (wywołane **po** fast metrics), więc lift się nie wykonywał.
- **Fix (follow-up):** Post-fallback open-quote lift dla **każdego** łańcucha z DB; w `node_metrics` dodany odczyt `open_quote` z ledgera + **live `current_value_usd`** dla otwartych PDA; w fast path live current dla otwartych; w post-fallback lift heurystyka także dla **otwartych** węzłów (`baseline < 85% open_quote`).
- **Guards/tests:** `cargo test -p clmm-lp-api open_quote_baseline_lift_post_fallbacks`
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`

---

### BUG-20260513-02 — PositionCreate: USDC saldo miga poprawnie, potem wraca do 0 (efektywne saldo / cache)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-13  
fixed_in: local  
keywords: PositionCreate, effective-balances, wallet_effective_cache, USDC, SPL, monotonic, degraded, force refresh, public RPC

- **Symptom:** Na `/positions/new` po swapie chwilowo widać sensowną sumę (SOL + USDC), po bezczynności **Stan portfela** pokazuje **0 USDC** mimo środków on-chain; walidacja „za mało środków” / stale wraca.
- **Root cause:** Tło odświeża `wallet_effective_cache` odczytem łańcucha, który potrafi zwrócić **pustą lub „zerową”** listę SPL przy nadal sensownym native SOL — zapis nadpisywał dobry snapshot gorszym.
- **Fix:** Monotoniczna straż przy zapisie do cache: pusta lista SPL względem poprzedniego snapshotu → zachowanie poprzednich wierszy tokenów; przy `confidence == degraded` uzupełnianie brakujących mintów i ochrona przed błyskiem „~0” względem sensownego poprzedniego salda; pełna regresja sald tylko przy `GET .../effective-balances?force=true` (`allow_balance_regression`).
- **Guards/tests:** `cargo test -p clmm-lp-api monotonic_guard`
- **Paths:** `crates/api/src/handlers/wallets.rs`, `doc/ENGINEERING_NOTES.md`

---

### BUG-20260513-01 — PositionCreate budget mode leaves Amount empty when public SOL/USD feed misses

status: partially fixed  
severity: medium  
reported_by: user  
first_seen: 2026-05-13  
fixed_in: local  
keywords: position-create, quote-open-budget, empty-amount, Missing USD price, SOL, WSOL, USDC, price-fetch, public-feed

- **Symptom:** `/positions/new` in USD budget mode shows `Bad request: Missing USD price for one or both pool mints; cannot size deposit`, and the bottom Amount SOL/USDC fields stay empty.
- **Observed evidence (2026-05-13):** Screenshot shows the exact backend 400 on `Last Candle 60m` position create while token Amount fields are blank; terminal history also shows public price feed instability (`GeckoTerminal` 429, Jupiter price misses for WSOL).
- **Symptom follow-up (2026-05-13):** After the source-level USDC-pool tick fallback was added and unit-tested, user still sees empty Amount SOL/USDC fields on the screen. Current hypothesis is not confirmed yet: either the running web proxy/API is still serving an older backend process, or the UI is not firing/accepting the budget quote for another reason.
- **Observed evidence (2026-05-13 follow-up):** Direct request to active API `127.0.0.1:8081` for pool `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE` with an in-range tick window returned valid quote values (`amount_a_ui=0.052410078`, `amount_b_ui=5.024187`, `estimated_value_usd~=10`). This shifts remaining suspicion from backend quote math to frontend query/state/refetch behavior.
- **Observed evidence (2026-05-13 follow-up 2):** User confirmed API had been restarted before the latest screenshot, so the remaining empty fields are not explained by an old backend process alone. Backend quote is healthy; remaining root cause is likely browser/frontend query state or disabled query conditions.
- **Symptom follow-up (2026-05-13 follow-up 3):** User reports the Amount SOL/USDC fields are still empty after the API restart and after the backend quote endpoint was verified healthy on port `8081`.
- **Symptom follow-up (2026-05-13 follow-up 4):** User confirms the bug still exists. Treat backend quote fallback as partial only; do not mark fixed until the actual browser/frontend path that leaves Amount empty is identified and verified.
- **Related prior bug family:** Same UX surface as `BUG-20260504-03` and `BUG-20260504-04` (`position-create`, `quote-open-budget`, empty Amount / stale caps), but this variant is caused by missing USD price data rather than out-of-range ticks or stale quote caps.
- **Root cause:** `quote_open_budget` required positive USD prices from `fetch_mint_prices_usd` for both pool mints. For SOL/USDC pools, if the public SOL/USD feed missed or was rate-limited, WSOL remained priced at `0`, so the endpoint rejected the quote even though the pool tick plus USDC=1 can derive a usable SOL/USD fallback.
- **Fix:** `quote_open_budget` now pins USDC/dev-USDC to `$1` and, for USDC pairs, derives the missing non-USDC leg price from `tick_current` and mint decimals before calling `quote_deposit_budget_in_range`.
- **Guards/tests:** `cargo test -p clmm-lp-api handlers::pools::tests::usdc_pool_fallback`
- **Paths:** `crates/api/src/handlers/pools.rs`, `doc/BUGS.md`, `doc/ENGINEERING_NOTES.md`

---

### BUG-20260512-05 — PositionCreate opens after insufficient confirmed swap-before-open

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-05-12  
fixed_in: local  
keywords: position-create, swap-before-open, open-position, effective-balances, insufficient-spl-balance, usdc, wsol, Orca

- **Symptom:** `/positions/new` shows green "Swap potwierdzony", then the open step fails with `open preflight: insufficient SPL balance on token B ... have 4785430 raw, need 4887945 raw` for USDC.
- **Observed evidence (2026-05-12):** API log for pool `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE` shows swap-only `specified_mint=So11111111111111111111111111111111111111112 amount_in=810376` raw, then subsequent open attempts. The confirmed swap was too small to cover the requested USDC cap.
- **Related prior bug family:** `BUG-20260504-07` also had "green swap confirmed, red open failed" in `PositionCreate`, but that case was stale balance blocking. This regression is the opposite: `handleSubmit` trusted any `swapSignature` and allowed open even when the post-swap funding check still showed a token deficit.
- **Root cause:** `PositionCreate.handleSubmit` treated a non-empty `swapSignature` as sufficient to bypass the token-deficit block. It did not require the wallet balance check to clear after swap-before-open, so a small/underfilled swap still led to backend open preflight.
- **Fix:** After swap-before-open succeeds, the UI invalidates and force-refreshes the API signer wallet balance. Submit now blocks if `fundingCheck.shortA/shortB` remains true after a confirmed swap and shows the remaining deficit instead of calling `POST /positions`.
- **Guards/tests:** `npx tsc --noEmit` in `web/`.
- **Paths:** `web/src/pages/PositionCreate.tsx`, `doc/BUGS.md`

---

### BUG-20260512-04 — Wallet effective balances disappear on API restart / first form load

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-12  
fixed_in: local  
keywords: wallet, effective-balances, PositionCreate, stale, warmup-placeholder, persistent-cache, CLMM_WALLET_EFFECTIVE_CACHE_PATH, force-refresh

- **Symptom:** `PositionCreate` can show stale/zero wallet state after API restart or first form load; "Wymuś odświeżenie" only invalidated frontend queries and could still return the same stale cache while the background refresh was pending.
- **Root cause:** `wallet_effective_cache` was in-memory only. Startup/resync only refreshed owners already present in memory, so the API signer was not guaranteed to have a last-good effective balance snapshot before the user opened the form.
- **Fix:** Effective wallet balances now hydrate from `data/wallet-effective-cache.json` (or `CLMM_WALLET_EFFECTIVE_CACHE_PATH`), every successful refresh writes an atomic public snapshot, startup/resync seeds the API/active signer, and `GET /wallets/effective-balances?force=true` performs a synchronized refresh used by the UI button.
- **Guards/tests:** `cargo test -p clmm-lp-api wallets -- --nocapture`; `npx tsc --noEmit` in `web/`.
- **Paths:** `crates/api/src/handlers/wallets.rs`, `crates/api/src/server.rs`, `crates/api/src/models.rs`, `web/src/pages/PositionCreate.tsx`, `web/src/lib/api.ts`, `doc/DATA_CATALOG.md`

---

### BUG-20260512-03 — Reopen silently downsizes position from ~$10 to ~$4

status: regressed  
severity: high  
reported_by: user  
first_seen: 2026-05-12  
fixed_in: local  
keywords: rebalance, reopen, silent-downsize, target_usd, open_position, token_caps, swap_mix, GgcTn1ij, 9danMXEL, AjKc9epD, Retouch shift

- **Symptom:** Rotation history for `GgcTn1ijVCcvPX1fHWUEFAqFBLWpeQhDbuUmwYkfVC9k` shows strategy notional dropping from about `$9.8` to about `$4.1`; user reports "znowu z 10 zrobiło się 4".
- **Observed evidence (2026-05-12):** Local API `/positions/GgcTn1ijVCcvPX1fHWUEFAqFBLWpeQhDbuUmwYkfVC9k` returns live `value_usd ~= 4.06`, so this is not only a UI render artifact. `/stream-lineage?mode=live` shows first shrink at `9danMXELSER2DxGiV4z1hoqAJVVii15dFQg73YjjKMMK` (`baseline_value_usd ~= 4.238`) after `AY746rx48R6jsXzrt8uCsBV4ksktM3nasbZaazWBdyK2` closed around `$9.760`.
- **Related prior bug family:** Similar product invariant to `BUG-20260510-01` (avoid `target_usd=0` / wallet-clamped downsizing) and `BUG-20260504-04` (~half notional opens when caps/quote are stale or incomplete), but this appears in automated rebalance/reopen rather than manual `PositionCreate`.
- **Root cause:** `open_new_range_with_wallet_mix` computes `target_usd` from previous close value, but before `open_position` it clamps each token cap independently to `min(wallet_cap, quote.token_max_*)` and did not re-check that the final caps still cover the quote. If swap-mix left one leg missing/stale, the open could proceed with only one quoted leg, producing an approximately half-sized position instead of failing into pending-open recovery.
- **Fix:** Added a final pre-open invariant after cap calculation: when a quote exists, effective caps (including native SOL for WSOL legs) must cover `q.amount_a/q.amount_b` within existing tolerance. If not, executor appends `bot_reopen_final_caps_below_target`, skips `open_position`, retries short refresh attempts, then returns a hard error for pending-open/recovery instead of opening undersized.
- **Guards/tests:** `cargo test -p clmm-lp-execution strategy::rebalance::tests::final_caps_guard_rejects_materially_undersized_quote_leg`
- **Paths:** `crates/execution/src/strategy/rebalance.rs`, `doc/BUGS.md`

---

### BUG-20260512-02 — Position history lineage loading waits ~30-40s on hot path

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-12  
fixed_in: local  
keywords: stream-lineage, position-history, rotations, loading-lineage, stream-performance, stream-pnl, lifecycle-ingest, position_stream_lineage, position_stream_performance, PositionDetail

- **Symptom:** `PositionDetail -> Historia pozycji (rotacje)` stays on `Ładowanie lineage…` for a long time.
- **Observed evidence (2026-05-12):** Local API timings for `HbpWqczFGQZkAEPzyQ2AJAm5vUpe41aKZ5CJdWfRTnLQ`: `/stream-performance` ~22-24s, `/stream-pnl?mode=live` ~23-25s, `/stream-lineage?mode=live` ~28-41s. For `85zg8khWUgRGT84VAw9xLddtm5tbqwRcftmVpndageLj`, `/stream-lineage` ~30s. Responses had `chain=1`, `nodes=1`, so the delay is backend work before/around lineage assembly, not a large rendered history.
- **Related prior bug family:** Stream-lineage has prior correctness fixes (`BUG-20260414-08`, `BUG-20260413-05`, chain-scope fixes), but this is a performance/hot-read regression.
- **Root cause:** `compute_position_stream_lineage` called `compute_position_stream_performance(state, entry, true)` to skip heavy JSONL->DB ingest, but then called `compute_position_stream_pnl(state, entry)`, whose implementation calls `compute_position_stream_performance(state, position_address, false)`. That reintroduced ingest/work on the lineage hot path. Direct `/stream-performance` and `/stream-pnl` also use `skip_ledger_ingest=false`, so their latency is dominated by repeated ingest/ledger work.
- **Fix:** `compute_position_stream_lineage` now reuses the already-computed `perf.positions` / `perf.sessions` and resolved lineage chain by calling `compute_position_stream_pnl_for_stream_members` directly. It no longer calls the public PnL wrapper that recomputes stream performance with ingest enabled.
- **Verification (2026-05-12):** Temporary API on port `18081` with the fix returned `/stream-lineage` for the same two PDAs in ~3.9s and ~6.0s (`chain=1`, `nodes=1`), down from ~30s.
- **Symptom (2026-05-12, follow-up):** User reports regression on `AjKc9epDjopLRx4QZS5BT9RXyAkNbgi1rY1aPYSGfyTs`: UI now shows `Request timed out in UI after 120s` for `/api/v1/positions/AjKc9epDjopLRx4QZS5BT9RXyAkNbgi1rY1aPYSGfyTs/stream-lineage?mode=live`; history does not render at all. This means the first fix removed one repeated-ingest cost but did not cover all slow paths.
- **Observed evidence (follow-up):** Direct local API call on `127.0.0.1:8081` returned but still slowly: `/stream-performance` ~24.3s, `/stream-pnl?mode=live` ~25.2s, `/stream-lineage?mode=live` ~41.9-54.6s. `AjKc...` resolves to `chain=12`, `nodes=12` (`3tD3...` -> `AjKc...`), unlike the prior verification (`chain=1`). A 20s proxy/API check times out on `8081`; UI timeout at 120s is plausible under load/background workers.
- **Root cause (follow-up):** `PositionDetail` launched `stream-performance`, `stream-pnl`, and `stream-lineage` concurrently for the same PDA. `stream-lineage` also still paid long-chain backend costs: per-node `node_metrics` repeated similar DB work, synchronous snapshot persist could contend with the read path, PnL totals could self-seed missing snapshots with on-chain reads, and WSOL price fallback spammed noisy public feeds.
- **Fix (follow-up):** `PositionDetail` now treats `/stream-lineage` as the primary stream history/totals request and enables `/stream-performance` + `/stream-pnl` only as fallback after lineage errors. Backend long-chain lineage now uses a batched DB snapshot/fee fast path for per-node metrics (`chain.len() > 8`), skips read-path snapshot persist for those long chains, disables PnL self-seed from lineage, adds short price timeouts, and stops WSOL from cascading through rate-limited fallback price feeds after CoinGecko.
- **Verification (follow-up, 2026-05-12):** Fresh API on port `18086` returned `/stream-lineage?mode=live` for `AjKc9epDjopLRx4QZS5BT9RXyAkNbgi1rY1aPYSGfyTs` (`chain=13`, `nodes=13`) in `3506ms`, then `1165ms` and `1277ms`, down from direct local measurements of ~42-55s.
- **Guards/tests:** Added invariant tests `stream_lineage_does_not_call_ingest_enabled_pnl_wrapper` and `stream_lineage_long_chains_use_batched_node_metrics`; verified with `cargo test -p clmm-lp-api stream_lineage -- --nocapture`, `cargo clippy -p clmm-lp-api --all-targets -- -D warnings`, `npx tsc --noEmit`, and IDE lints for `web/src/pages/PositionDetail.tsx` + `web/src/lib/api.ts`.
- **Residual risk:** For long chains, per-node rows use the fast snapshot/tx-fee batch and expose detailed collect/cashflow through chain totals rather than recomputing every detailed per-node collect leg on the UI hot path. If the UI later needs exact per-node collect/cashflow for every historical node, add a separate drill-down endpoint or background materialization.
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`, `crates/api/src/services/position_stream_pnl.rs`, `crates/api/src/services/price_fetch.rs`, `crates/api/src/services/position_stream_performance.rs`, `web/src/pages/PositionDetail.tsx`

---

### BUG-20260512-01 — Regression: running strategies skip out-of-range linked positions missing from registry

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-12  
fixed_in: local  
keywords: regression, strategy-executor, managed_allowlist, position_addresses, registry_open, out_of_range, monitor_in_range, diagnostics, no-rebalance, retouch_shift, last_candle, auto_execute

- **Symptom:** Dashboard shows linked strategies with `auto_execute=true`, `running=true`, and positions `out_of_range`, but bot does not close/reopen; no `Decision requires action` appears for the affected PDAs.
- **Observed evidence (2026-05-12):** `HbpWqczFGQZkAEPzyQ2AJAm5vUpe41aKZ5CJdWfRTnLQ` (`Retouch shift`) and `85zg8khWUgRGT84VAw9xLddtm5tbqwRcftmVpndageLj` (`Last Candle 60m`) are in `data/strategies.json` and live `/strategies`, while `/positions` reports `in_range=false`; `/positions/{pda}/diagnostics` has no `last_eval`, and `data/positions/registry.jsonl` has no matching `registry_open`.
- **Related prior bug family:** This is a regression/variant of the older `in_range` + strategy-link/allowlist issues: 2026-04-06 engineering note (“diagnostics + fresh `in_range`”), `BUG-20260414-04` (empty `position_addresses` / managed allowlist semantics), `BUG-20260414-05` (divergent executor wiring + managed allowlist/reopen hook), and `BUG-20260415` lineage/link follow-ups around `position_addresses` continuity.
- **Root cause:** `managed_allowlist_pubkeys_for_strategy_parameters` intersected configured `position_addresses` with `registry_open_position_pubkeys()`. Operator/API-opened positions can be present in strategy config and monitor but absent from registry, making the executor allowlist exclude them before evaluation.
- **Fix:** For explicit non-empty `parameters.position_addresses`, allow configured valid PDAs directly instead of requiring `registry_open`; missing/non-array still falls back to registry-open legacy behavior, and explicit `[]` remains restrictive (“manage nothing”).
- **Guards/tests:** Added regression `configured_position_addresses_do_not_require_registry_open`; verified with `cargo test -p clmm-lp-api configured_position_addresses_do_not_require_registry_open -- --nocapture`, `cargo test -p clmm-lp-api managed_allowlist -- --nocapture`, and `cargo clippy -p clmm-lp-api --all-targets -- -D warnings`.
- **Paths:** `crates/api/src/services/strategy_service.rs`, `crates/execution/src/strategy/executor.rs`, `crates/api/src/handlers/positions.rs`

---

### BUG-20260511-01 — CI `make lint` fails on strict clippy after rebalance recovery commit

status: fixed  
severity: medium  
reported_by: monitoring  
first_seen: 2026-05-11  
fixed_in: local  
keywords: ci, github-actions, make-lint, clippy, collapsible_if, too_many_arguments, manual_range_contains, manual_clamp, clmm-lp-execution

- **Symptom:** GitHub Actions `Lint` failed for commit `f99bd64` with `Process completed with exit code 2`; `clmm-lp-execution` was rejected by `-D warnings`.
- **Symptom (local full lint):** After the first clippy fixes, full workspace lint also surfaced `manual_range_contains` in a rebalance unit test and `redundant_locals` in `uncollected_fees_cache`.
- **Root cause:** The rebalance/pending-open recovery changes introduced strict clippy violations: nested `if`, two helper functions above the argument threshold, manual range/clamp patterns, and a redundant local captured into spawned tasks.
- **Fix:** Collapsed the pending-open stale-session check, grouped SOL-first wallet inputs into a helper struct, replaced manual range/clamp patterns with clippy-compliant forms, and removed the redundant task-capture local.
- **Guards/tests:** `cargo clippy -p clmm-lp-execution -- -D warnings`; `cargo clippy --all-targets --all-features -- -D warnings`
- **Paths:** `crates/execution/src/strategy/executor.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/api/src/services/uncollected_fees_cache.rs`

---

### BUG-20260510-01 — `no_close_unless_reopen_feasible` stuck: `target_usd=0` when wallet SPL empty (funds only in LP)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-10  
fixed_in: local  
keywords: rebalance, reopen_preflight, bot_reopen_preflight_failed, no_close_unless_reopen_feasible, target_usd, wallet_notional, prev_end_value_usd, OOR, close+reopen, clmm-lp-execution

- **Symptom:** Positions stay open OOR; lifecycle repeats `bot_reopen_widen_ticks` then `bot_reopen_preflight_failed` with `wa`/`wb` = 0, `wallet_notional` = 0, `prev_end_value_usd` > 0, `target_usd` = 0 — no `bot_close_position`.
- **Root cause:** Reopen preflight (before close) used `target_usd_from_prev_end_clamped(prev_end, wallet_notional)` with **pre-close** SPL balances. All value in LP → empty ATAs → clamp to 0 → `quote_deposit_budget_in_range` rejects (`target_usd` must be > 0).
- **Fix:** Preflight uses `target_usd_for_close_reopen_preflight`: budget caps against estimated **post-close** spendable `wallet_notional + prev_end_value_usd` (same synthetic prices), then `min` with `prev_end_value_usd` and 0.995 margin.
- **Guards/tests:** `cargo test -p clmm-lp-execution preflight_target_usd --lib`
- **Paths:** `crates/execution/src/strategy/rebalance.rs`

---

### BUG-20260506-03 — Vite WS proxy errors (`ECONNABORTED` / `ECONNRESET`) despite healthy API

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-05-06  
fixed_in: local  
keywords: web, vite, websocket, ws-proxy, ECONNABORTED, ECONNRESET, dashboard, api-v1, proxy-rewrite

- **Symptom:** Vite dev server prints repeated WebSocket proxy errors:
  - `[vite] ws proxy error: Error: write ECONNABORTED`
  - `[vite] ws proxy error: Error: read ECONNRESET`
  even while the dashboard loads and the API is reachable.
- **Evidence:** API `GET /api/v1/health` returns HTTP 200 on `127.0.0.1:8081`; WS upgrade to `/api/v1/ws/positions` returns HTTP 101 (Switching Protocols).
- **Root cause:** Multi-factor:
  - API applied `tower_http::timeout::TimeoutLayer` to the versioned base router under `/api/v1`, which also wrapped `/api/v1/ws/*`. The timeout cancels the upgraded request task and resets the underlying socket after ~\(timeout\) seconds, surfacing as `ECONNRESET` in the Vite proxy and `code=1006` (abnormal closure) in the browser.
  - Client reconnect loop was triggered even on intentional `disconnect()` (e.g., dev StrictMode mount/cleanup/mount), causing extra churn and making the resets look “constant”.
- **Fix:**
  - Align WS routing: Vite dev proxy rewrites `/ws/*` → `/api/v1/ws/*` to match versioned API routes.
  - Add forensics logging to correlate disconnects after-the-fact (see below).
  - WebSocket client: avoid duplicate connections while `CONNECTING` and disable auto-reconnect after intentional `disconnect()` (prevents churn).
  - API: exclude `/api/v1/ws/*` routes from timeout layers by composing versioned routers via separate `nest("/api/v1", ...)` boundaries; keep timeouts on REST routes only.
- **Guards/tests:** N/A (dev-only proxy behavior; diagnosed via logs + manual reproduction).
- **Forensics (added):**
  - Vite WS proxy logs: `tools/logs/vite-ws-proxy.log` (proxy req/error/open/close)
  - Browser WS logs: `localStorage["ws_debug_log_v1"]` (close `code/reason/wasClean`)
  - Disable: `VITE_WS_PROXY_LOG=0` and/or `VITE_WS_CLIENT_LOG=0` in `web/.env.local` (restart Vite)
- **Paths:** `web/vite.config.ts`, `web/src/lib/websocket.ts`, `crates/api/src/routes.rs`

---

### BUG-20260506-02 — `LastCandlePeriodic` ignored user interval (rebalance every eval tick)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-06  
fixed_in: local  
keywords: last_candle_periodic, LastCandlePeriodic, min_rebalance_interval_minutes, periodic_interval_minutes, cooldown, rebalance-loop, strategies, decision-engine

- **Symptom:** Strategy type `Last candle (periodic)` rebalanced much more frequently than the user-configured interval (e.g. rebalance after ~4 minutes even though UI/strategy parameters were set to 45 minutes). Observed cadence matched executor eval tick (~5m).
- **Root cause:** `StrategyMode::LastCandlePeriodic` gated on `DecisionConfig.min_rebalance_interval_minutes`, but periodic-like interval clamping and semantics are expressed via `DecisionConfig.periodic_interval_minutes`. When the min interval was effectively `0` (or stale), the periodic gate was bypassed.
- **Fix:** `LastCandlePeriodic` now uses `periodic_interval_minutes` for its time gate (same as `Periodic`). Added regression test to ensure it does not rebalance before the periodic interval even if `min_rebalance_interval_minutes=0`.
- **Guards/tests:** `cargo test -p clmm-lp-execution strategy::decision --lib`
- **Paths:** `crates/execution/src/strategy/decision.rs`

---

### BUG-20260506-01 — Pending-open/reopen stuck on `insufficient native SOL` despite WSOL/USDC in wallet

status: fixed  
severity: critical  
reported_by: user  
first_seen: 2026-05-06  
fixed_in: local  
keywords: pending-open, reopen, insufficient-native-sol, sol-first, wsol, usdc, open_position, preflight, rebalance

- **Symptom:** After bot closes a position and enqueues pending-open recovery, reopen repeatedly fails with `open preflight exact-plan: insufficient native SOL ...` even though the wallet has funds in WSOL and/or USDC. Operator sees closed positions stuck in “awaiting reopen”.
- **Root cause:** SOL-first WSOL auto-unwrap only ran **after** successful txs; the pending-open open attempt fails **before** any unwrap/swap can run (native SOL is checked in preflight), so the system never converts available WSOL/USDC into native SOL for rent/fees.
- **Fix:** In reopen/pending-open open loop (`open_new_range_with_wallet_mix`), detect the preflight error and perform a one-shot **operational native SOL top-up** before retrying open: unwrap WSOL→native SOL if present; if still short, swap minimal **stable (pool leg) → WSOL** in-pool and unwrap to native SOL (stable mint = other leg vs WSOL, or `CLMM_STABLE_MINT_FOR_SOL_TOPUP`). Then retry `open_position`.
- **Guards/tests:** `cargo test -p clmm-lp-execution --lib`; `cargo test -p clmm-lp-api --lib` (includes ignored devnet matrix tests `devnet_strategy_workflow_*`).
- **Paths:** `crates/execution/src/strategy/rebalance.rs`

---

### BUG-20260505-04 — Fees zebrane overstated: close rows lacked `fee_owed` snapshot; principal leaked into fees

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-05  
fixed_in: local  
keywords: fees, collect_fees, close_position, fee_owed, lp_collected_token_raw, position_stream_lineage, logs-rebalances, PositionDetail

- **Symptom:** In `Logs / rebalances` (Position Detail), the `LP zebrane`/fees column showed values that did not match actual LP fees; could appear inflated because close principal was counted as fees.
- **Symptom (2026-05-06 regression):** `Fees zebrane` shows `—/0` for positions closed today; lifecycle `bot_close_position` rows miss `lp_collected_token_{a,b}_raw`, so API cannot compute authoritative fees from close events.
- **Symptom (2026-05-06):** Some positions show **large USDC “Fees zebrane”** that actually equals the **principal+fees** returned on close (`fee_payer_token_deltas`), while lifecycle `lp_collected_token_{a,b}_raw` exists but is **`0/0`**. Solscan shows non-zero “Claim fees” for the same close tx.
- **Symptom (2026-05-12):** `GET /positions/{address}/stream-pnl` returned `INTERNAL_ERROR: column "lp_collected_token_a_raw" does not exist` on databases where collected-fee raw legs are stored only inside `position_stream_ledger_rows.raw_json`.
- **Symptom (2026-05-12 regression):** After stream-lineage/logs speedup, `Logs / rebalances -> Fees zebrane` renders all zeros for long rotation chains because the fast per-node path returns `fees_collected_* = 0/None`.
- **Root cause:** Lifecycle `bot_close_position` rows did not carry an authoritative `fee_owed_a/b` snapshot, and historical data often lacked `details.close_amount_*_raw`, so API fallback subtraction could not remove principal from `fee_payer_token_deltas`.
- **Root cause (added):** `position.fee_owed_{a,b}` read **before** close can be stale/zero unless Whirlpool `update_fees_and_rewards` has been applied; close instructions compute fees via update+quote (as shown by Solscan / Orca SDK `feesQuote`), so persisting the pre-close account fields produced `0/0` even when “Claim fees” was non-zero.
- **Fix:** Persist close fees using Orca SDK quote (`close_position_instructions(...).fees_quote.fee_owed_{a,b}`) rather than pre-close `position.fee_owed_{a,b}`. API treats `lp_collected_token_*_raw=0/0` on close as **non-authoritative** and falls back to close-subtraction (principal isolation) to avoid counting principal as fees.
- **Fix (2026-05-12):** Stream PnL / lineage fee rollups now keep LP fees as first-class components (`realized_lp_fees_usd`, `uncollected_lp_fees_usd`) and the DB collect-fee helper also treats `lp_collected_token_*_raw=0/0` as non-authoritative. Close-event fee legs are valued with `details.event_price_*_usd` when available, falling back to current free prices only when event prices are missing.
- **Fix (2026-05-12 follow-up):** The DB collect-fee helper reads `lp_collected_token_*_raw` from `raw_json` only, so it works on both older and current local DB schemas that do not have dedicated raw-leg columns.
- **Fix (2026-05-12 regression):** Long-chain fast lineage keeps the batched snapshot path but now adds a batched ledger fee rollup for all PDAs in the chain (`position_pubkey = ANY(chain)`) and fills per-node `fees_collected_usd`, token legs, and `collect_events`; lifecycle JSONL fallback is used only for nodes where DB fee rows are empty/sparse.
- **Fix (UI):** Renamed `LP zebrane` → `Fees zebrane` and renamed the multiplier from `collects` to neutral `events`.
- **Guards/tests:** `cargo test -p clmm-lp-execution --lib`; `cargo check -p clmm-lp-protocols`; `cargo check -p clmm-lp-api`; `cargo test -p clmm-lp-api position_stream_pnl`; `cargo test -p clmm-lp-api stream_lineage -- --nocapture`; `cargo clippy -p clmm-lp-api --all-targets -- -D warnings`; `npx tsc --noEmit` in `web/`.
- **Paths:** `crates/execution/src/strategy/rebalance.rs`, `crates/protocols/src/orca/executor.rs`, `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/api/src/services/position_stream_pnl.rs`, `crates/api/src/services/position_stream_lineage.rs`, `crates/api/src/models.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionDetail.tsx`, `web/src/components/PositionLifecycleTimeline.tsx`

---

### BUG-20260506-03 — Reopen swap fails with `BlockhashNotFound` due to endpoint mismatch (blockhash vs send)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-06  
fixed_in: local  
keywords: rpc, blockhashnotfound, send_and_confirm, current_endpoint, endpoint-rotation, swap_exact_in_failed, reopen, pending-open, publicnode

- **Symptom:** After `bot_close_position`, bot starts swap-mix and fails on first swap with `simulation_err=UiTransactionError(BlockhashNotFound)`, leaving position stuck closed without a successful reopen/open. Lifecycle row shows `rpc_url` on one endpoint, while error string shows `send_transaction failed (endpoint=...)` on another.
- **Root cause:** Transaction is signed with a recent blockhash fetched via provider, but `send_and_confirm_transaction` iterated over `all_endpoints()` (fan-out) and could send the signed tx to a different RPC fleet that did not recognize the blockhash yet, resulting in `BlockhashNotFound`.
- **Fix:** Pin send+confirm to `current_endpoint()` for the whole attempt; rotate endpoint only **between** attempts (provider-level), avoiding cross-endpoint blockhash/send mismatch.
- **Guards/tests:** `cargo check -p clmm-lp-protocols -p clmm-lp-execution`
- **Paths:** `crates/protocols/src/rpc/provider.rs`, `crates/protocols/src/orca/executor.rs`, `crates/execution/src/strategy/rebalance.rs`

---

### BUG-20260506-04 — Watchdog cannot auto-enqueue stranded sessions when only `planned_new_tick_*` exists

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-06  
fixed_in: local  
keywords: stranded-rebalance-watchdog, pending-open, reconcile, planned_new_tick_lower, planned_new_tick_upper, bot_close_position, intended_ticks

- **Symptom:** `POST /bot-activity/stranded-rebalances/reconcile` reports stranded sessions (close seen, open missing) but returns `can_auto_enqueue=false` with note “Missing IL rebalance_incomplete row; watchdog can report but cannot infer intended ticks.” even though lifecycle `bot_close_position.details` contains `planned_new_tick_lower/upper`.
- **Root cause:** Watchdog only extracted fallback ticks from `bot_recover_open_replanned.details` (`new_tick_*` / `intended_tick_*`) and ignored the common close-row rotation plan keys `planned_new_tick_lower/upper`.
- **Fix:** Watchdog now accepts `planned_new_tick_lower/upper` as valid tick hints and considers `bot_close_position` rows for fallback hint extraction.
- **Guards/tests:** `cargo check -p clmm-lp-api`
- **Paths:** `crates/api/src/services/stranded_rebalance_watchdog.rs`

---

### BUG-20260505-05 — Pending-open swap-mix fails when WSOL deficit can be covered by native SOL (wrap not preferred)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-05  
fixed_in: local  
keywords: swap-mix, pending-open, wsol, native-sol, wrap, quote_deposit_budget_in_range, rebalance

- **Symptom:** Pending-open recovery repeated with `swap mix: exhausted 10 rounds without matching deposit quote` while wallet had sufficient **native SOL** to cover the WSOL leg; bot performed unnecessary USDC→WSOL swaps.
- **Evidence:** Swap-mix diagnostics showed `wa=0` (WSOL SPL), but `wa_ui>0` and `wallet_notional` large (native SOL present), `deficit_a>0` on WSOL leg, and `leg=B_to_A` swaps executed.
- **Root cause:** Swap-mix preferred in-pool swaps when `wa==0` even if token A is WSOL and the wallet had spendable native SOL; the pre-wrap path covered WSOL-on-B and some other branches but missed WSOL-on-A deficit.
- **Fix:** In swap-mix, when token A is WSOL and `deficit_a>0` with `wa<=MIN_SWAP` and spendable native SOL exists, pre-wrap native SOL into WSOL ATA and retry quote before swapping the other leg.
- **Guards/tests:** `cargo check -p clmm-lp-execution`; `cargo test -p clmm-lp-execution swap_mix_sol_first`.
- **Paths:** `crates/execution/src/strategy/rebalance.rs`

---

### BUG-20260505-03 — WSOL pre-wrap failed with ATA `IllegalOwner`, blocking reopen and causing repeated swap attempts

status: fixed  
severity: critical  
reported_by: user  
first_seen: 2026-05-05  
fixed_in: local  
keywords: wsol, ata, illegalowner, createidempotent, pending-open, rebalance, swap-mix, reopen

- **Symptom:** Rebalance sessions showed repeated `bot_swap_mix_*` / `bot_swap_exact_in_*` attempts with no successful reopen for the same closed position; pending-open queue kept retrying.
- **Symptom (session evidence):** `pending-open-recovery.json` contained `last_error: swap-mix wsol pre-wrap ... InstructionError(0, IllegalOwner)` with ATA program in instruction index 0.
- **Root cause:** WSOL pre-wrap path used non-idempotent ATA create semantics in a race-prone read-then-create flow; when ATA appeared between read/send (or RPC lagged), wrap tx could fail with ATA `IllegalOwner` before open attempt.
- **Fix:** Switched WSOL ATA creation to `create_associated_token_account_idempotent`. Added fallback path on ATA `IllegalOwner`: re-validate ATA (`program owner`, mint, token owner), then retry topup-only transfer + `sync_native` without ATA create.
- **Guards/tests:** Added unit tests `test_is_ata_illegal_owner_error_matches_expected_shape` and `test_is_ata_illegal_owner_error_ignores_other_failures`; verified with `cargo check -p clmm-lp-protocols`.
- **Paths:** `crates/protocols/src/orca/executor.rs`

---

### BUG-20260505-02 — `LP zebrane` counted only `bot_collect_fees`, not total realized fees (collect + close)

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-05-05  
fixed_in: local  
keywords: stream-lineage, fees_collected_usd, bot_collect_fees, bot_close_position, close_amount_raw, ClosedPositionDetail

- **Symptom:** In chain history, first positions showed `LP zebrane = 0` while operator expected fees realized at close to be included; only manual collect on the last position appeared.
- **Root cause:** API fee aggregation in `position_stream_lineage` filtered strictly to `event='bot_collect_fees'`; close rows were excluded even though they carry fee-bearing deltas.
- **Fix:** `fees_collected_*` now aggregates fee legs from **collect + close** rows. For close rows, principal leg is removed by subtracting `details.close_amount_{a,b}_raw` (or DB `raw_json`) from positive pool-leg deltas, leaving best-effort fee remainder.
- **Fix (UI copy):** Renamed table/card wording from `LP zebrane` to `Fees zebrane`; event counter caption is neutral (`× events`) instead of `× collect`.
- **Guards/tests:** `cargo check -p clmm-lp-api`; `npx tsc --noEmit`.
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`, `web/src/pages/ClosedPositionDetail.tsx`, `web/src/lib/api.ts`

### BUG-20260505-01 — API strategies without `il_ledger_path` miss `rebalance_incomplete` rows; watchdog cannot auto-enqueue

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-05  
fixed_in: local  
keywords: api, strategy_executor, il_ledger_path, rebalance_incomplete, stranded-rebalances, pending-open-recovery, bot_recover_open_replanned

- **Symptom:** `stranded-rebalances` showed `close_seen=true`, `open_seen=false`, repeated swap/replan events, but `rebalance_incomplete_logged=false` and `can_auto_enqueue=false` (`Missing IL rebalance_incomplete row...`) for running API strategies.
- **Root cause:** API `start_strategy_executor_core` set IL ledger path only from optional `strategy.parameters.il_ledger_path`; UI-created strategies usually omit this field, so `LifecycleTracker::record_rebalance_incomplete` had nowhere to append.
- **Root cause (follow-up):** Watchdog inferred intended ticks only from IL `rebalance_incomplete` rows; when IL row was missing but lifecycle had `bot_recover_open_replanned.details.new_tick_*`, it still refused auto-enqueue.
- **Fix:** API now defaults IL ledger path to `CLMM_IL_LEDGER_PATH` or `data/ledger/il-ledger.jsonl` and creates parent dirs best-effort. Watchdog adds lifecycle fallback hints from `bot_recover_open_replanned.details` (`new_tick_*` / `intended_tick_*`) and can auto-enqueue with fallback note.
- **Guards/tests:** `cargo test -p clmm-lp-api stranded_rebalance_watchdog`; `cargo check -p clmm-lp-api`.
- **Paths:** `crates/api/src/services/strategy_service.rs`, `crates/api/src/services/stranded_rebalance_watchdog.rs`

### BUG-20260504-08 — Bollinger / last-candle bot: new range can exclude live `tick_current` → position immediately OOR

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-04  
fixed_in: local  
keywords: bollinger, last_candle, StrategyExecutor, record_price_and_compute_bollinger_ticks, tick_current, out_of_range, whirlpoolTicks

- **Symptom:** Opened ~10 USD target but notional lower; position shows `out_of_range` right after open while live price sits just outside a tight Bollinger band derived from rolling samples.
- **Root cause:** `web/src/lib/whirlpoolTicks.ts` expands aligned ticks to include `current_tick` before USD quote / open; `record_price_and_compute_bollinger_ticks` and `record_price_and_compute_last_closed_candle_ticks` in `executor.rs` did not — historical band can lag a fast move.
- **Fix:** Shared `expand_spacing_aligned_range_to_include_current_tick` after spacing alignment; unit tests for expand behavior.
- **Guards/tests:** `cargo test -p clmm-lp-execution expand_tick_range`.
- **Paths:** `crates/execution/src/strategy/executor.rs`

### BUG-20260504-07 — PositionCreate: open blocked on stale balances right after confirmed swap-before-open

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-05-04  
fixed_in: local  
keywords: position-create, swap-before-open, effective-balances, is_stale, handleSubmit, open-position

- **Symptom:** Green “Swap potwierdzony” then red “Otwarcie nieudane” / stale-balance message (~30s stale) despite user attempting open; UI told them to force-refresh even after on-chain swap succeeded.
- **Root cause:** `handleSubmit` rejected whenever `effectiveBalancesQ.data.is_stale`; post-swap invalidation can still return `is_stale` from projection/fast-return while chain state is already updated.
- **Fix:** If `swapBeforeOpen` and a non-empty `swapSignature` exist, do not block open on `is_stale` (swap is authoritative for that step); keep the guard for all other flows. Bilingual stale error via `L(...)`.
- **Guards/tests:** `npx tsc --noEmit` in `web/`.
- **Paths:** `web/src/pages/PositionCreate.tsx`

### BUG-20260504-06 — Rebalance: many swap-mix txs but no reopen (stale pool tick/√P for open quote)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-04  
fixed_in: local  
keywords: rebalance, swap-mix, reopen, open_position, quote_deposit_budget_in_range, tick_current, sqrt_price, pool_state, WhirlpoolReader

- **Symptom:** Session showed multiple successful in-pool swaps / swap-mix rounds under a rebalance UUID but no `bot_open_position` (or open failed after retries) while mix had converged.
- **Root cause:** `ensure_swap_mix_for_rebalance_open` refetches `WhirlpoolState` every mix round; `open_new_range_with_wallet_mix` used the **post-close** `pool_state` snapshot for `quote_deposit_budget_in_range` (`tick_current`, `sqrt_price`) and synthetic price — after swaps, on-chain √P/tick diverged from that snapshot → wrong deposit caps vs chain.
- **Fix:** Before each open attempt, refetch pool state via `WhirlpoolReader::get_pool_state` and use that for mints, price, tick, √P in the open loop; ledger `details` include `open_quote_pool_tick_current` / `open_quote_pool_sqrt_price` for ops.
- **Guards/tests:** `cargo check -p clmm-lp-execution`.
- **Paths:** `crates/execution/src/strategy/rebalance.rs`

### BUG-20260504-05 — Swap-mix ledger diagnostics missing rebalance_session_id (`_no_session` in UI)

status: fixed  
severity: low  
reported_by: user  
first_seen: 2026-05-04  
fixed_in: local  
keywords: rebalance, swap-mix, ledger, rebalance_session_id, closed-position, diagnostics

- **Symptom:** Closed position timeline showed `bot_swap_mix_round` / `bot_swap_exact_in_attempt` under `_no_session` while `bot_swap_exact_in` txs grouped under a UUID session (e.g. after close without reopen).
- **Root cause:** `try_append_bot_diagnostic_row(..., None, ...)` in `ensure_swap_mix_for_rebalance_open`; real swaps already carried `ledger_session_id`.
- **Fix:** Pass `ledger_session_id.clone()` for all swap-mix diagnostic rows (including `bot_swap_mix_failed`).
- **Guards/tests:** `cargo check -p clmm-lp-execution`.
- **Paths:** `crates/execution/src/strategy/rebalance.rs`

### BUG-20260504-04 — USD budget open: ~10 USD target but position ~half notional (stale caps vs ticks)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-04  
fixed_in: local  
keywords: position-create, quote-open-budget, token_max, budgetSubmitRaw, open-position, bollinger

- **Symptom:** Manual open with “~10 USD” budget; on-chain / UI position value ~\$5.6 (example PDA `CDrYCk3CDfUzxM4QCkMaY61pdLBKhXBU4kLjTPH7fuVR`).
- **Root cause:** `budgetSubmitRaw` could stay aligned with an **older** `quote-open-budget` response while `tick_lower` / `tick_upper` auto-synced (e.g. Bollinger band + expand) — POST sent **smaller** `token_max_*` than the quote shown for the latest range.
- **Fix:** Drive submit + funding caps from current `budgetQuoteQ.data` only; block submit while quote refetching; reject `in_range=false`; warn when `estimated_value_usd` is well below typed USD.
- **Guards/tests:** `npx tsc --noEmit` in `web/`.
- **Paths:** `web/src/pages/PositionCreate.tsx`

### BUG-20260504-03 — Bollinger on PositionCreate: no USD quote / no swap hint (ticks off live price)

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-05-04  
fixed_in: local  
keywords: position-create, bollinger, tick-range, out-of-range, quote-open-budget, swap-before-open, whirlpoolTicks

- **Symptom:** With Bollinger strategy + USD budget mode, orange “cena poza zakresem”, empty Amount fields, no in-app swap proposal despite balances.
- **Root cause:** Bollinger ticks from snapshot history did not always contain `current_tick`; `budgetQuoteEnabled` stayed false → no amounts → `fundingCheck` never ready.
- **Fix:** `expandAlignedTickRangeToIncludeCurrent` after band alignment; clearer out-of-range copy linking empty Amount to missing swap hints.
- **Guards/tests:** `npx tsc --noEmit` in `web/`.
- **Paths:** `web/src/lib/whirlpoolTicks.ts`, `web/src/pages/PositionCreate.tsx`

### BUG-20260504-02 — Bot swap-mix ignored native-only SOL (SPL zeros) and WSOL-on-B wrap gap

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-05-04  
fixed_in: local  
keywords: clmm-lp-execution, rebalance, swap-mix, wsol, sol-first, native-lamports, swap_before_open

- **Symptom:** With only native SOL (no SPL WSOL/USDC), rebalance swap-mix bailed early or never pre-wrapped; WSOL as pool token B lacked the A-leg-only wrap path.
- **Root cause:** `wa==0 && wb==0` hard error; `wallet_notional` ignored native; `can_wrap_native_sol_for_wsol_leg` only when `token_mint_a == WSOL`.
- **Fix:** Allow SPL-zero continue when pool has WSOL + spendable lamports; SOL-first UI amounts for notional; symmetric pre-wrap for WSOL on A or B; unit tests `swap_mix_sol_first_tests`.
- **Guards/tests:** `cargo test -p clmm-lp-execution swap_mix_sol_first`.
- **Paths:** `crates/execution/src/strategy/rebalance.rs`

### BUG-20260504-01 — PositionCreate showed 0 SOL for WSOL leg despite native SOL

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-05-04  
fixed_in: local  
keywords: position-create, wsol, sol-first, native-sol, wallet-line, effective-balances, formatBalanceLine, is-stale

- **Symptom:** On `/positions/new`, the SOL side of the pool displayed `0` (or only SPL WSOL) while Portfel showed ~0.726 native SOL; label read like SOL but reflected empty WSOL ATA.
- **Root cause:** `formatBalanceLine` only read `balances.tokens` for the WSOL mint; native SOL lives in `balances.sol`.
- **Fix:** For WSOL mint, display `balances.sol` as primary; if SPL WSOL > 0, append note with SPL amount. Block position submit while `is_stale` on effective balances.
- **Symptom (2026-05-04, follow-up):** With `is_stale` true but non-zero cached SOL, deficit/Jupiter hints for missing USDC disappeared (user: SOL-only, ~10 USD open).
- **Root cause (2026-05-04, follow-up):** `fundingCheck` returned `ready: false` for any stale read, hiding the whole funding banner.
- **Fix (2026-05-04, follow-up):** Only suppress funding when stale **and** balances look like all-zero warmup; otherwise compute deficits; small banner note when stale.
- **Symptom (2026-05-04, follow-up 2):** In-app „swap w puli Orca przed open” (checkbox + `swap_before_open` API) not offered when SOL leg was native-only (WSOL SPL = 0).
- **Root cause (2026-05-04, follow-up 2):** `swapBeforeOpenPlan` used `getAvailableUiAmount(WSOL)` for max input → 0 raw.
- **Fix (2026-05-04, follow-up 2):** Use `fundingCheck.effectiveHaveA` / `effectiveHaveB` for swap-in cap (aligned with SOL-first funding).
- **Guards/tests:** `npx tsc --noEmit` in `web/`.
- **Paths:** `web/src/pages/PositionCreate.tsx`

### BUG-20260430-03 — PositionCreate Bollinger did not auto-set range from Bollinger bands

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-30  
fixed_in: local  
keywords: position-create, bollinger, range, ticks, snapshots, strategy, ui

- **Symptom:** On `/positions/new`, choosing strategy `Bollinger` still used static width around current tick (`range_width_pct`) instead of deriving bounds from current Bollinger bands.
- **Root cause:** `PositionCreate` only read `parameters.range_width_pct` for strategy auto-sync; Bollinger band calculation existed in executor runtime (for rebalance decisions) but not in open-form UX.
- **Fix:** Added frontend Bollinger auto-range in `PositionCreate`: fetches recent pool snapshots (`GET /data/snapshots`), computes `mean ± k*sigma` over `bollinger_window` prices, aligns to pool `tick_spacing`, and uses those ticks for auto-sync when `strategy_type=bollinger`.
- **Symptom (2026-04-30, follow-up):** UI still reported `Brak wystarczających danych` despite active 5m/10m snapshots and healthy data coverage for the pool.
- **Root cause (2026-04-30, follow-up):** `GET /data/snapshots` returned `price_ab` as optional, but many snapshot rows only had `tick_current`; frontend Bollinger used `price_ab` only and discarded valid rows.
- **Fix (2026-04-30, follow-up):** API snapshots handler now derives `price_ab` from `tick_current` via `tick_to_price` when `price_ab` is missing, so Bollinger range can use existing snapshot streams without new collectors.
- **Symptom (2026-04-30, follow-up 2):** On open form users still saw confusing `0 SOL`/partial balances while their selected wallet had funds, especially during stale windows; flow looked blocked without clear action.
- **Root cause (2026-04-30, follow-up 2):** Validation uses API signer balances, while user mentally compares selected wallet balances; stale read state lacked explicit recover action on form.
- **Fix (2026-04-30, follow-up 2):** `PositionCreate` now displays selected-wallet quick balance (informational) when it differs from API signer and adds `Wymuś odświeżenie` action in stale banner to invalidate/refetch relevant wallet balance queries.
- **Guards/tests:** `npm run build` in `web/` passes.
- **Paths:** `web/src/pages/PositionCreate.tsx`, `web/src/lib/api.ts`

### BUG-20260430-02 — Positions page: `fetch_positions_for_owner` 403 from PublicNode → internal error

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-30  
fixed_in: local  
keywords: positions-ui, orca, positions-by-owner, fetch_positions_for_owner, publicnode, 403, SOLANA_RPC_URL, bad-gateway

- **Symptom:** Internal error mentioning `fetch_positions_for_owner: HTTP status client error (403 Forbidden) for url (https://solana.publicnode.com/)` when loading Positions (on-chain Orca scan).
- **Root cause:** `/orca/positions-by-owner` built a single `RpcClient` from `current_endpoint()` only, so it did not use the same multi-endpoint policy as `RpcProvider`’s fallbacks; a blocked/limiting public RPC failed the whole request.
- **Fix:** Handler tries each URL from `all_endpoints()` until success; on total failure returns **502** with guidance to set `SOLANA_RPC_URL` / `SOLANA_RPC_FALLBACK_URLS`.
- **Paths:** `crates/api/src/handlers/orca_onchain.rs`, `doc/ENGINEERING_NOTES.md`

### BUG-20260430-01 — PositionCreate: stale effective-balances warmup blocked „Za mało tokenów” / open

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-30  
fixed_in: local  
keywords: position-create, effective-balances, is-stale, react-query, refetch-interval, funding-validation, warmup-placeholder

- **Symptom:** On `/positions/new`, wallet lines stayed at 0 SOL / 0 USDC for minutes with stale banner; funding validation treated placeholder zeros as real and blocked open („Za mało tokenów…”).
- **Root cause:** After fast-return warmup (`is_stale=true`), `PositionCreate` did not poll like Wallet; React Query could sit on placeholder data and `fundingCheck` treated zeros as deficits.
- **Fix:** Aggressive `refetchInterval` when stale or missing data (~2.5s) vs ~10s when fresh; `refetchOnMount: 'always'`; invalidate `wallet-balances` when `effectiveOwnerPk` changes; skip token/SOL funding validation while `is_stale`; submit shows explicit „balances refreshing” instead of false insufficient-funds.
- **Guards/tests:** `npm run build` in `web/` (tsc + vite); manual: cold load `/positions/new` — banner may flash but open must not false-block once balances settle.
- **Paths:** `web/src/pages/PositionCreate.tsx`

### BUG-20260429-03 — Swap: partial WSOL->SOL unwrap failed + mixed-language raw errors + low dark-mode contrast

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-29  
fixed_in: local  
keywords: swap, wsol, sol, unwrap, partial, i18n, error-message, dark-theme, contrast

- **Symptom:** In PL locale, `Swap -> Convert WSOL <-> SOL` displayed English raw backend error (`partial unwrap is not supported...`) and conversion failed for non-Max WSOL->SOL amounts. Error readability on dark theme remained poor in multiple pages (`text-destructive` on very dark backgrounds).
- **Root cause:** Backend unwrap path supported only full WSOL ATA close and explicitly rejected partial amounts. Frontend error path rendered raw API messages without normalization/localization. Dark theme destructive token was too dark for widespread `text-destructive` usage and many pages used weak `bg-destructive/5..10`.
- **Fix:** Added partial unwrap support in Orca executor (`close -> re-wrap remainder` flow), added API-side frontend message normalization for known SOL/WSOL failures in `messageFromErrorBody`, updated Swap UI copy (partial supported) and error banners, and increased dark-theme destructive token contrast globally.
- **Symptom (follow-up):** On Swap screen user saw only "Konwersja wysłana", and source WSOL temporarily appeared as `0` after partial unwrap despite not using Max, making final outcome unclear.
- **Symptom (follow-up 2):** `SOL -> WSOL` behaved like "set WSOL target balance" instead of converting the entered amount delta, which could make conversion appear incorrect versus user expectation.
- **Root cause (follow-up):** API convert response exposed only a single "submitted" signature/message while partial unwrap uses multi-step execution semantics; UI had no explicit final confirmation state and no delayed balance refresh for chained tx visibility.
- **Root cause (follow-up 2):** `native_to_wsol` path called `submit_wsol_wrap_with_signature_if_needed(req.amount_raw)` where argument means target WSOL ATA amount, not delta to wrap.
- **Fix (follow-up):** Convert API now reports confirmed outcome metadata (`confirmed`, `partial`, step signatures fields) and Swap UI renders final confirmed status + step signatures with short post-conversion balance refetch loop and partial-RPC warning when token read is degraded.
- **Fix (follow-up 2):** Added delta wrap path in Whirlpool executor (`submit_wsol_wrap_with_signature_delta`) and switched API `native_to_wsol` conversion to wrap exactly requested amount; response now includes post-conversion balances (`post_native_lamports`, `post_wsol_raw`) for deterministic UI confirmation.
- **Fix (follow-up 3):** Added operation ledger + reconciliation for `convert-sol` (`op_id`, `reconciliation_status`) with background verifier and status endpoints (`/wallets/ops`, `/wallets/ops/{op_id}`), so UI can distinguish `confirmed_unreconciled` vs `reconciled`/`mismatch` under RPC instability.
- **Fix (follow-up 4):** Added adaptive hedging + token-bucket budget for idempotent wallet reads, extended reconciliation diagnostics (`reason_code`, `attempts`, `last_verified_at_utc`), and new aggregate telemetry endpoint (`/wallets/ops/stats`) to monitor mismatch/unreconciled pressure.
- **Guards/tests:** `npx tsc --noEmit` (web), `cargo check -p clmm-lp-protocols -p clmm-lp-api`, `cargo test -p clmm-lp-protocols test_compute_unwrap_rewrap_amount -- --nocapture`.
- **Guards/tests (follow-up 3/4):** `cargo test -p clmm-lp-api wallets::tests:: -- --nocapture`, `cargo check -p clmm-lp-api`, `npx tsc --noEmit`.
- **Paths:** `crates/protocols/src/orca/executor.rs`, `crates/api/src/handlers/wallets.rs`, `crates/api/src/models.rs`, `crates/api/src/server.rs`, `crates/api/src/routes.rs`, `crates/api/src/state.rs`, `crates/api/src/openapi.rs`, `web/src/lib/api.ts`, `web/src/pages/Swap.tsx`, `web/src/pages/Wallet.tsx`, `web/src/index.css`

---

### BUG-20260429-02 — `snapshot-run-curated-all` omitted Meteora vault fields needed for auto `lp_share`

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-29  
fixed_in: local  
keywords: backtests, snapshot-run-curated-all, meteora, lp_share, vault_amount_a, vault_amount_b, token-2022, snapshot-only

- **Symptom:** Meteora rows in Backtests FULL often failed with `Meteora snapshot-only: set --lp-share ... include vault_amount_a/vault_amount_b`, even after fresh snapshot cycles.
- **Root cause:** Meteora branch inside `snapshot-run-curated-all` serialized `vault_amount_a/vault_amount_b` as optional fields and skipped them when decode returned `None`; decode path used strict SPL unpack only, so Token-2022/extended account layouts frequently produced missing vault amounts.
- **Fix:** In `snapshot-run-curated-all`, aligned Meteora vault decode behavior with curated collector: added token-account fallback decoder (extension-friendly), made `vault_amount_a/vault_amount_b` always present (`u64`) with explicit `vault_amount_source` (`rpc_token_account` or `missing_fallback_zero`).
- **Guards/tests:** `cargo check -p clmm-lp-cli` (targeted compile guard for CLI snapshot path).
- **Paths:** `crates/cli/src/main.rs`

---

### BUG-20260429-01 — Strategy Create forced `dry_run=true` with no visible toggle

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-29  
fixed_in: local  
keywords: strategies, create, dry_run, ui, strategy-create, auto-execute

- **Symptom:** Przy tworzeniu strategii użytkownik nie widział opcji `Dry run`; nowa strategia była zawsze zapisywana z `dry_run=true`, więc trzeba było wejść w edycję i ręcznie odznaczać.
- **Root cause:** `StrategyCreate` wysyłał payload z hardcoded `dry_run: true` i `auto_execute: false`, bez sekcji checkboxów znanej z `StrategyEdit`.
- **Fix:** Dodano kontrolki `Dry run` i `Auto-execute` do formularza tworzenia; payload korzysta teraz z wartości z UI zamiast stałych.
- **Guards/tests:** `npx tsc --noEmit` (web) + lint na dotkniętych plikach.
- **Paths:** `web/src/pages/StrategyCreate.tsx`

---

### BUG-20260427-05 — Mixed PL/EN labels and tiny helper text reduced UI readability

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-27  
fixed_in: local  
keywords: web, i18n, mixed-labels, readability, font-size, dashboard, settings, strategies, position-detail

- **Symptom:** UI still showed mixed Polish/English labels on multiple screens (`Dashboard`, `Closed positions`, `Strategies`, `Create Strategy`, `Settings`, `PositionDetail`) and many helper/diagnostic lines were too small to read comfortably.
- **Root cause:** Previous i18n migration covered selected flows only; several pages still had hardcoded literals. Additionally, the app used many `text-xs` / `text-[10px]` / `text-[11px]` utilities in dense diagnostic cards.
- **Fix:** Added bilingual labels on key screens and normalized remaining hardcoded strings in `PositionDetail`; added global readability uplift for smallest text utilities in `web/src/index.css`.
- **Guards/tests:** `npx tsc --noEmit` (web) + lint pass on touched frontend files.
- **Paths:** `web/src/index.css`, `web/src/pages/Dashboard.tsx`, `web/src/pages/ClosedPositions.tsx`, `web/src/pages/Strategies.tsx`, `web/src/pages/StrategyCreate.tsx`, `web/src/pages/Settings.tsx`, `web/src/pages/PositionDetail.tsx`, `doc/ENGINEERING_NOTES.md`

---

### BUG-20260427-04 — Wallet balances endpoint omitted Token-2022 accounts

status: partially fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-27  
fixed_in: local  
keywords: wallet, balances, token-2022, spl, getTokenAccountsByOwner, rpc, web

- **Symptom:** Po wejściu na `Wallet` część tokenów nie pojawiała się na liście on-chain mimo poprawnego owner pubkey i działającego RPC/SOL.
- **Root cause:** API `GET /wallets/balances` czytało token accounts tylko dla programu SPL Token legacy (`Tokenkeg...`), pomijając konta Token-2022 (`TokenzQd...`).
- **Fix:** Endpoint pobiera teraz token accounts z obu programów (`Tokenkeg...` + `TokenzQd...`), scala wynik per `mint` (sumowanie `ui_amount`) i zachowuje dotychczasowe fallbacki (SOL zwracany nawet przy częściowej niedostępności token RPC).
- **Symptom (follow-up):** Po wdrożeniu odczytu z obu programów UI nadal potrafi pokazać mniej tokenów niż oczekiwane; toggle `Pokaż zera` nie zmienia listy.
- **Root cause (follow-up):** Endpoint `wallets/balances` pracuje w trybie partial-success: gdy któryś call RPC (`legacy` albo `token-2022`) nie powiedzie się, API nadal zwraca `200` z tokenami tylko z działającej gałęzi. UI wcześniej nie pokazywał, że wynik jest częściowy.
- **Fix (follow-up):** API zwraca teraz status diagnostyczny obu odczytów (`token_legacy_ok`, `token_2022_ok`, `token_*_error`, `token_accounts_total`), a Wallet UI pokazuje ostrzeżenie „lista może być niepełna” z konkretnym statusem/błędem RPC.
- **Symptom (new):** Użytkownik widzi `http 403 Forbidden` z `solana.publicnode.com` (`blocked parameter: params.1.programId`) dla `getTokenAccountsByOwner` oraz sporadyczne błędy sieciowe; oba odczyty mogą być `legacy=false | token-2022=false`, mimo że saldo SOL działa.
- **Root cause (new):** Część darmowych RPC blokuje wywołania `getTokenAccountsByOwner` z filtrem `programId` (legacy/Token-2022). Obecna diagnostyka zwracała tylko ostatni błąd z listy endpointów, co zaciemniało który endpoint i dlaczego nie zadziałał.
- **Symptom (newer):** Mimo skonfigurowanych fallbacków API nadal często zwraca `tokens=[]` (`legacy=false`, `token-2022=false`) — oba odczyty kończą timeoutem.
- **Root cause (newer):** `wallets/balances` uruchamiało pojedynczy read per program-id z twardym timeoutem; retry/failover mogły nie zdążyć przejść sensownej liczby endpointów w oknie czasu.
- **Fix (newer):** Dodano fanout first-success-wins per program-id (`CLMM_WALLET_BALANCES_FANOUT`, domyślnie 3): API odpytuje kilka endpointów równolegle i bierze pierwszy sukces. W błędach zwracana jest telemetria prób endpointów.
- **Symptom (latest):** `Wallet` naprzemiennie pokazuje pełną listę tokenów i chwilowe `tokens=[]` (`confidence=degraded`) mimo aktywnego virtual-wallet/WS path; użytkownik obserwuje „flapping” co kilka odświeżeń.
- **Root cause (latest):** Odczyt baseline token accounts nadal był „edge-triggered” na bieżącej jakości public RPC. Przy chwilowym `403/timeout` oba programy mogły zwrócić fail i endpoint nie utrzymywał ostatniego poprawnego snapshotu tokenów; dodatkowo fanout próbował penalizowane endpointy ponownie bez krótkoterminowej pamięci błędów.
- **Fix (latest):** Dodano owner-scoped fallback `last-good token snapshot` w `effective-balances` (używany gdy oba odczyty token programs fail i bieżąca lista jest pusta) oraz mechanizm penalizacji endpointów token-account (`CLMM_WALLET_TOKEN_ENDPOINT_PENALTY_SECS`) dla błędów `403/429/timeout`, aby fanout czasowo omijał niestabilne RPC.
- **Symptom (latest-2):** `GET /wallets/effective-balances` w UI potrafiło timeoutować po 15s na zimnym starcie ownera (`cache miss`), mimo że endpoint miał być szybkim read-model path.
- **Root cause (latest-2):** Przy `cache miss` handler wykonywał synchroniczne `compute_effective_balances` (pełny on-chain read) zamiast natychmiastowego zwrotu i tła.
- **Fix (latest-2):** Wymuszono hard fast-return na `cache miss`: endpoint zwraca od razu placeholder `degraded` (`is_stale=true`) i uruchamia refresh wyłącznie w tle; stale cache zwracane z metadanymi (`is_stale`, `stale_age_ms`).
- **Guards/tests (latest):** testy jednostkowe dla penalizacji endpointów (`penalize_token_endpoint_marks_and_filters`, `penalize_token_endpoint_ignores_non_penalty_errors`) + `cargo check -p clmm-lp-api`.
- **Guards/tests (latest-2):** `cargo check -p clmm-lp-api`, `npm run build`, lint diagnostics dla zmienionych plików API/Web.
- **Guards/tests:** Dodano regresyjny unit test `merge_wallet_token_rows_sums_same_mint`.
- **Paths:** `crates/api/src/handlers/wallets.rs`, `crates/api/src/models.rs`, `web/src/lib/api.ts`, `web/src/pages/Wallet.tsx`, `.env.example`

---

### BUG-20260427-03 — Close Position returned opaque Whirlpool custom 6005

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-27  
fixed_in: local  
keywords: close-position, whirlpool, custom-6005, closepositionnotempty, not-empty, liquidity, fees, rewards

- **Symptom:** Manual close failed with `InstructionError(3, Custom(6005))` on Whirlpool ix (`whirLb...`) and API returned generic bad-request chain without clear remediation.
- **Symptom (session evidence):** Error payload included `custom_code=6005` on `/positions/{pda}` close attempt from Position Detail UI.
- **Root cause:** `classify_close_position_error` in API had dedicated branches for `3007` and `6018`, but no explicit mapping for Whirlpool `6005` (`ClosePositionNotEmpty`), so operators got opaque output.
- **Fix:** Added explicit close error mapping for `custom 6005` with actionable hint: position is not empty yet (remaining liquidity and/or unsettled fee/reward legs) and close should be retried after state refresh/settlement.
- **Guards/tests:** Added regression test `close_position_error_6005_maps_to_bad_request_with_not_empty_hint`.
- **Paths:** `crates/api/src/services/position_service.rs`

---

### BUG-20260427-01 — Position history `range @ close` was blank despite close ticks present

status: fixed  
severity: low  
reported_by: user  
first_seen: 2026-04-27  
fixed_in: local  
keywords: position-detail, range-close, old_tick_lower, old_tick_upper, lifecycle, web

- **Symptom:** W `PositionDetail -> Position history` kolumna `range @ close` pokazywała `—` mimo że event close miał ticki w details.
- **Root cause:** Frontend parser czytał wyłącznie `details.tick_lower/tick_upper`; close rows często zapisują zamykany zakres jako `old_tick_lower/old_tick_upper`, więc parser nie widział danych.
- **Fix:** Rozszerzono parser zakresów w `PositionDetail`: open czyta `tick_*` i fallback `new_tick_*`, close czyta `tick_*` i fallback `old_tick_*`.
- **Guards/tests:** `npx tsc --noEmit` w `web/`.
- **Paths:** `web/src/pages/PositionDetail.tsx`

---

### BUG-20260427-02 — Monitored positions table showed stale/zero PnL from monitor cache

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-27  
fixed_in: local  
keywords: positions, monitored-positions, pnl, fees, source-of-truth, stream-pnl, fallback-monitor

- **Symptom:** W `Positions -> Monitored positions (API)` kolumna `PnL` często pokazywała `0.000%` lub nieadekwatne wartości, mimo aktywnej pozycji i zmian na `Position Detail`. Użytkownik pytał też, czy `Fees` są z właściwego źródła.
- **Root cause:** Lista pozycji renderowała `PnL` z `position.pnl.net_pnl_pct` (monitor cache), które nie jest wiarygodnym live source dla tej tabeli. `Fees` były liczone poprawnie z valuation path (`fees_earned_usd`), ale UI nie pokazywał jawnie źródła (`live_valuation` vs `fallback_monitor`), więc zera wyglądały jak błąd.
- **Fix:** `Positions` pobiera teraz per-wiersz `stream-pnl` i używa `net_pnl_pct` z tego endpointu jako priorytetowego źródła. Gdy stream nie jest dostępny, zostaje fallback do monitor cache z etykietą źródła. Przy `Fees` dodano czytelny znacznik źródła valuation.
- **Guards/tests:** `npx tsc --noEmit` (web), ręczna weryfikacja: PnL na liście zgadza się kierunkiem/skalą z `Position Detail -> stream`.
- **Paths:** `web/src/pages/Positions.tsx`

---

### BUG-20260427-01 — Position history ranges used raw tick ratio (wrong scale for token pair)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-27  
fixed_in: local  
keywords: position-history, rotations, range-open, range-close, tick-to-price, decimals, ui

- **Symptom:** W `Position history (rotations)` kolumny `range @ open` / `range @ close` pokazywały nielogiczne zakresy (np. `0.085... USDC per 1 SOL` zamiast ~`85...`), mimo poprawnych ticków.
- **Root cause:** UI konwertowało tick -> `tickToPriceRatio` (raw `B_raw/A_raw`) i wyświetlało wynik bez korekty o decymale tokenów (`10^(decA-decB)`), więc zakres był przeskalowany.
- **Fix:** W `PositionDetail` zakres ticków jest teraz liczony jako raw ratio -> UI ratio przez `uiPriceFromRawPriceRatio`, z decimalami z Orca token metadata (`getOrcaToken`) i fallbackiem dla znanych mintów/labeli (USDC/USDT/SOL/BTC/ETH).
- **Guards/tests:** `npx tsc --noEmit` (web), ręczna weryfikacja na `/positions/{pda}`: zakresy dla SOL/USDC są w skali dziesiątek/setek, nie setnych.
- **Paths:** `web/src/pages/PositionDetail.tsx`, `web/src/lib/whirlpoolTicks.ts`

---

### BUG-20260424-05 — Wallet page showed SOL immediately but SPL tokens appeared only after manual refresh

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-24  
fixed_in: local  
keywords: wallet, spl-tokens, rpc-timeout, refresh, retry, ux, web

- **Symptom:** Po wejściu na Wallet widać SOL, ale lista tokenów SPL bywa pusta; tokeny pojawiają się dopiero po ręcznym odświeżeniu strony.
- **Root cause:** API endpoint `/wallets/balances` zwraca sukces z pustą listą tokenów, gdy call `getTokenAccountsByOwner` chwilowo nie przejdzie (SOL jest zwracany niezależnie). UI traktował taki stan jako finalny i nie ponawiał automatycznie.
- **Fix:** Added bounded auto-retry in Wallet UI when owner is set, request succeeded, and token list is empty (up to 4 retries, delayed), plus explicit retry status message.
- **Guards/tests:** `npx tsc --noEmit` (web); manual verify: first empty token response triggers automatic retries without page refresh.
- **Paths:** `web/src/pages/Wallet.tsx`

---

### BUG-20260424-04 — Static manual lower/upper was treated as width proxy, not absolute bounds

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-24  
fixed_in: local  
keywords: backtests, static, manual-range, lower-upper, absolute-bounds, api, cli

- **Symptom:** User set `static_manual_lower=85` and `static_manual_upper=90` for SOL/USDC, but result rows still showed different static ranges (e.g. ~83-88, ~85-90) across windows.
- **Root cause:** API converted manual bounds into a derived `% width` and pinned `min/max-range-pct`, so engine still anchored ranges around each window entry price instead of using absolute bounds.
- **Fix:** Added dedicated CLI args `--static-manual-lower` / `--static-manual-upper`, passed through API only for valid single-pool manual runs, and wired backtest engine to apply these as absolute initial bounds for `StratConfig::Static` only.
- **Guards/tests:** `cargo check -p clmm-lp-cli -p clmm-lp-api`; rerun FULL backtest with one pool and manual static range should keep static bounds fixed to entered levels.
- **Paths:** `crates/api/src/handlers/backtests.rs`, `crates/cli/src/main.rs`, `crates/cli/src/backtest_engine.rs`

---

### BUG-20260424-03 — Auto-Tune loop marked completed FULL jobs as failed (`done` vs `succeeded/partial`)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-24  
fixed_in: local  
keywords: auto-tune, backtests, full-run, status, succeeded, partial, done, api

- **Symptom:** `Status: running | note: Full optimize cycle failed` appeared even when FULL jobs were completing and returning results.
- **Root cause:** Auto-Tune polling logic expected terminal success status `"done"`, but `start_backtest_full` writes `"succeeded"` or `"partial"`. This caused success/partial cycles to be treated as failures.
- **Fix:** Updated Auto-Tune success branch to accept `"succeeded"` and `"partial"` as completed cycles; it now stores latest winner when results exist and sets note to either completed or completed (partial).
- **Guards/tests:** `cargo check -p clmm-lp-api`; manual verification via `/backtests/auto-tune/status` note progression after FULL cycle.
- **Paths:** `crates/api/src/handlers/backtests.rs`

---

### BUG-20260424-02 — Backtests FULL failed with `unexpected argument 'true'` for threshold OOR flag

status: partially fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-24  
fixed_in: local  
keywords: backtests, full-run, threshold, cli, clap, bool-flag, unexpected-argument

- **Symptom:** FULL runs ended as `partial` with CLI parse error: `unexpected argument 'true' found` for `--threshold-rebalance-on-range-exit-immediately`.
- **Symptom (follow-up):** Error persisted in runtime when API still resolved an older `clmm-lp-cli` binary that exposed switch-only syntax for this flag.
- **Root cause:** API passed the threshold OOR toggle as `--threshold-rebalance-on-range-exit-immediately true/false`, while older CLI variants parsed it as a pure boolean switch (set-true flag) and rejected explicit value tokens.
- **Fix:** 
  - CLI parser changed to `ArgAction::Set` for `threshold_rebalance_on_range_exit_immediately` (accepts explicit `true/false`).
  - API now probes `backtest-optimize --help` and auto-selects argument style:
    - value style (`... true/false`) when supported,
    - switch style (flag only for `true`) for older binaries.
  - For older binaries and explicit `false`, API skips the flag and logs warning (falls back to CLI default).
- **Guards/tests:** `cargo check -p clmm-lp-cli -p clmm-lp-api`; verify `backtest-optimize --help` and rerun FULL job with threshold toggle.
- **Paths:** `crates/cli/src/main.rs`, `crates/api/src/handlers/backtests.rs`

---

### BUG-20260424-01 — Backtests: static manual lower/upper looked unusable (inputs disabled by default)

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-24  
fixed_in: local  
keywords: backtests, static, manual-range, lower-upper, ux, disabled-input, multi-pool

- **Symptom:** Użytkownik nie mógł wpisać wartości do pól `static_manual_lower` / `static_manual_upper` na stronie Backtests.
- **Root cause:** Inputy były fizycznie zablokowane (`disabled`) dopóki nie była wybrana dokładnie 1 para, a domyślnie zaznaczonych jest wiele par.
- **Fix:** Inputy są teraz zawsze edytowalne; przy wielu parach wartości manual range pozostają wpisywalne, ale UI jasno komunikuje, że nie zostaną użyte i obowiązuje `static_deviation_pct`.
- **Guards/tests:** `npx tsc --noEmit` (web), weryfikacja UX: wpisanie `lower/upper` przy multi-pool nie blokuje edycji i pokazuje komunikat.
- **Paths:** `web/src/pages/Backtests.tsx`

---

### BUG-20260423-02 — Backtests FULL: `oor_recenter` vs `retouch_shift` looked like identical strategies

status: fixed  
severity: low  
reported_by: user  
first_seen: 2026-04-23  
fixed_in: local  
keywords: backtests, full-run, oor_recenter, retouch_shift, run_single, strategy-help, ui

- **Symptom:** W rankingu FULL `oor_recenter` i `retouch_shift` potrafily pokazywac te same (lub bardzo zblizone) metryki.
- **Root cause:** W `crates/cli/src/backtest_engine.rs` obie strategie sa **idle**, dopoki cena nie wyjdzie z pasma — przy calym oknie **in-range** zachowuja sie jak static (0 rebalansow). Przy **jednym** epizodzie OOR na plateau (stala cena poza starym pasmem) czesto wystarcza **jeden** rebalance dla kazdej, wiec koszty/score moga sie zrownac po zaokragleniu; pelna roznica ujawnia sie przy **monotonicznym** uciekaniu ceny po OOR: `OorRecenter` moze rebalance'owac na kolejnych krokach, `RetouchShift` rzadziej (bronia `retouch_armed` + geometria krawedzi).
- **Fix:** Rozszerzono opisy w `STRATEGY_HELP` (Backtests) o semantyke symulacji; dodano testy regresyjne `oor_recenter_matches_retouch_shift_when_price_never_leaves_initial_band` oraz `oor_recenter_rebalances_more_often_than_retouch_on_monotonic_climb_after_oor` w `crates/cli/src/engine/tests.rs`.
- **Guards/tests:** `cargo test -p clmm-lp-cli oor_recenter_`.
- **Paths:** `web/src/pages/Backtests.tsx`, `crates/cli/src/engine/tests.rs`

### BUG-20260423-01 — Backtests FULL: HTTP 422 when grid CSV sent floats into u64 JSON fields

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-23  
fixed_in: local  
keywords: backtests, full-run, 422, unprocessable, periodic_grid_steps, bollinger_window_grid, last_candle_seconds_grid, serde, web

- **Symptom:** Klikniecie `Uruchom FULL porownanie` na stronie Backtests zwracalo `HTTP 422` (body requestu odrzucone).
- **Root cause:** `BacktestFullRequest` w API uzywa `Vec<u64>` dla m.in. `periodic_grid_steps`, `bollinger_window_grid`, `last_candle_*_grid`; UI parsowal wszystkie siatki jako `number[]` i serializowal ulamki (np. `0.5` w periodic), co lamalo deserializacje Axum/Serde.
- **Fix:** W `Backtests.tsx` rozdzielono parser CSV: ulamki tylko dla `threshold_grid_pct` i `bollinger_k_grid`; calkowite dla pozostalych gridow (niecalkowite tokeny pomijane). Domyslne wartosci formularza dopasowano do oczekiwanego zestawu (periodic jako calkowite kroki).
- **Guards/tests:** `npx tsc --noEmit` w `web/`.
- **Paths:** `web/src/pages/Backtests.tsx`

### BUG-20260422-03 — Rebalance open/recovery could reuse dust sizing and open near-zero positions

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-22  
fixed_in: local  
keywords: rebalance, recover-open, dust, open-target-usd, close-amounts, lifecycle-ledger

- **Symptom:** Rotation chain could jump from normal values (e.g. ~$2.4) to `start/end ~$0.000` and then continue rebalancing on dust PDAs.
- **Symptom (session evidence):** In the same lineage/session, close rows carried non-zero value, but subsequent `bot_open_position` was sized as near-zero.
- **Symptom (2026-05-06 evidence):** Po `close -> open` nowa pozycja potrafila otworzyc sie na **ulamek** poprzedniej kwoty (np. leg USDC `open_amount_b_raw` rzedu `136`), gdy po close wallet mial **native SOL**, ale WSOL SPL balance/ATA bylo `0` — open sizing/caps liczyl tylko SPL `wa/wb` i mogl wejsc w dust fallback.
- **Symptom (follow-up):** In `Position history`, node could still show `start/open ~4 USD` while the same PDA `current_value` was near zero, because continuity rewrote `baseline_value_usd` from previous node end.
- **Root cause:** Open sizing in rebalance used pre-close calculated `amount_*_before_calc` (which can be stale/tiny in edge flows) instead of authoritative close amounts. Recovery path `recover_open_after_incomplete` hardcoded `amount_a_before_raw=1` and `amount_b_before_raw=1`, forcing `prev_end_value_usd` and `target_usd` toward dust.
- **Root cause (follow-up):** Session continuity in lineage (`close(old)->open(new)` by `rebalance_session_id`) always overwrote node baseline with `prev_end`, even when node baseline was already computed from open-row data (`open_amount_raw` / caps path).
- **Fix:** Standard rebalance open now passes `close_amount_a_raw`/`close_amount_b_raw` from `read_close_amounts_best_effort` into `open_new_range_with_wallet_mix`. Recovery open now loads latest matching close amounts from lifecycle close rows (`details.close_amount_*_raw`) by `position_pubkey` and optional `rebalance_session_id`, falling back to legacy `1,1` only when no row is found. In lineage continuity, baseline from session is now applied only when node baseline is missing (`0`), so explicit open-derived baseline is preserved. Additionally, open sizing/caps now use **SOL-first** logic for the WSOL leg (native SOL counted when WSOL ATA balance is 0), matching swap-mix and upstream Whirlpool bot patterns.
- **Guards/tests:** Added unit test `close_amounts_from_lifecycle_row_parses_matching_close` and `continuity_from_session_does_not_override_existing_baseline`; verified with `cargo check -p clmm-lp-execution`, `cargo check -p clmm-lp-api`, and (2026-04-27 follow-up) `cargo test -p clmm-lp-api --no-run` after fixing stale `DecisionConfig.periodic_interval_hours` -> `periodic_interval_minutes` in `devnet_e2e_tests`.
- **Paths:** `crates/execution/src/strategy/rebalance.rs`, `crates/api/src/services/position_stream_lineage.rs`

---

### BUG-20260422-02 — Position lineage cashflow included open/close principal; rebalance preflight spammed zero collect rows

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-22  
fixed_in: local  
keywords: position-lineage, cashflow, net-pnl, fee_payer_token_deltas, open-close-principal, collect_fees, reopen-preflight

- **Symptom:** In `Position history (rotations)`, per-node `cashflow`/`net PnL` could look inflated/unintuitive (e.g. large positive cashflow despite small `start->end` NAV delta), because principal `open/close` token legs were mixed into cashflow.
- **Symptom:** `Logs / rebalances` showed repeated sessions with `bot_collect_fees` where `A raw: 0, B raw: 0`, followed by many `bot_reopen_widen_ticks`/`bot_reopen_preflight_failed` diagnostic rows.
- **Root cause:** Per-node DB lineage path aggregated `fee_payer_token_deltas` for cashflow without filtering lifecycle `open/close` events. Separately, rebalance flow executed `collect_fees_first` before reopen-feasibility guardrail; when preflight failed, close/open was skipped but zero-fee collect tx was already emitted.
- **Fix:** Lineage cashflow now excludes principal legs by filtering out lifecycle open/close rows in DB path (`non-principal` cashflow only). Rebalance flow no longer performs preflight-time `collect_fees_first`; collection is kept on close paths.
- **Guards/tests:** `cargo check -p clmm-lp-execution -p clmm-lp-api`; unit regression in decision engine to ensure strategy loop does not emit standalone `CollectFees`.
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/decision.rs`

---

### BUG-20260422-01 — Rebalance session could emit two `bot_open_position` rows; second PDA stayed orphaned

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-22  
fixed_in: local  
keywords: rebalance, duplicate-open, rebalance-session-id, strategy-link, orphan-position, logs

- **Symptom:** A single `rebalance_session_id` could contain two `bot_open_position` rows (e.g. `7FW...` and `85A...`), while strategy link/heal followed only one path (`old -> new`), leaving the second position orphaned from strategy management.
- **Root cause:** Open/link flow assumes one successful open per session (`new_position: Option<Pubkey>` + one reopen hook call). There was no explicit guardrail to block duplicate open execution for an already-opened session id.
- **Fix:** Added open guard in rebalance executor: before open, block when lifecycle already has `bot_open_position` for session id, or when session is already inflight/completed in-process; emit diagnostic `bot_open_guard_blocked`. Added session helper in lifecycle ledger reader for `session_has_bot_open_position`.
- **Symptom (2026-04-27, follow-up):** Guard blocked duplicate attempts (`bot_open_guard_blocked`), but duplicate triggers still appeared in logs for the same session (`session_open_inflight_or_completed`), indicating concurrent workers were still attempting recovery/open.
- **Root cause (2026-04-27, follow-up):** Pending-open recovery processing had no cross-executor claim/lease for the same session item, so parallel executor loops could race on one `rebalance_session_id` and rely on late open guard rejection. Additionally, strategy start path could replace executor instances without explicitly stopping/removing any pre-existing one first.
- **Fix (follow-up):** Added global pending-open claim key (`sid:<rebalance_session_id>`; fallback `pool+closed_position`) so only one worker processes a recovery item at a time; non-claiming workers keep item untouched (no extra attempts). Added defensive replacement guard in `start_strategy_executor_core` to stop+remove any existing executor instance before spawning a fresh one for the same strategy id.
- **Fix (observability/UI):** Added tick-range context to close rows (`old_tick_*`, `planned_new_tick_*`) and previous-range context to open rows (`prev_tick_*`, `new_tick_*` in details), then rendered side-by-side graphical range panels in Logs session view.
- **Guards/tests:** `cargo check -p clmm-lp-execution`; `npx tsc --noEmit` (in `web/`).
- **Paths:** `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/executor.rs`, `crates/api/src/handlers/strategies.rs`, `web/src/pages/Logs.tsx`

---

### BUG-20260421-04 — Backtests TOP3 per family showed duplicate strategy labels with different ranges

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-21  
fixed_in: local  
keywords: backtests, top3, strategy-family, duplicates, range, bollinger, ui

- **Symptom:** In `Strategie spelniajace target`, a family section (e.g. `bollinger`) could show the same strategy label multiple times (e.g. `bollinger_w10_k1_r24`) with slightly different ranges.
- **Root cause:** Family-level TOP3 selection sorted all variants and sliced first 3 rows, but did not deduplicate by `strategy` label before slicing.
- **Fix:** Added deduplication in `qualifyingTop3`: after sorting family rows by selected metric, keep only the best row per `strategy` label, then take TOP3.
- **Guards/tests:** `npx tsc --noEmit` in `web/`.
- **Paths:** `web/src/pages/Backtests.tsx`

---

### BUG-20260421-03 — Backtests FULL failed on stale CLI missing optimize-grid flags

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-21  
fixed_in: local  
keywords: backtests, full-run, backtest-optimize, threshold-grid-pct, stale-cli, api, feature-probe

- **Symptom:** Web Backtests FULL run failed for all pools/windows with `exit Some(2): unexpected argument '--threshold-grid-pct' found`.
- **Root cause:** API always passed optimize-grid override flags (`--threshold-grid-pct`, etc.) whenever request fields were present, but capability probing guarded only `--include-strategy-families`. When API resolved an older `clmm-lp-cli` binary, Clap rejected unknown grid flags.
- **Fix:** Added `backtest-optimize --help` probe for requested grid flags before matrix execution; API now fails fast with explicit rebuild/`CLMM_LP_CLI_PATH` guidance when any requested grid flag is unsupported.
- **Guards/tests:** `cargo check -p clmm-lp-api`.
- **Paths:** `crates/api/src/handlers/backtests.rs`

---

### BUG-20260421-02 — Position Agent tab looked inactive (quick actions were non-clickable, fallback was opaque)

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-21  
fixed_in: local  
keywords: position-agent, quick-actions, ui, llm-fallback, chat, position-detail

- **Symptom:** In `PositionDetail -> Position Agent`, quick actions (`scan_now`, `compare_7d_ranges`, etc.) were visible but did not do anything, so tab looked “dead”.
- **Symptom:** Sending prompts often returned generic fallback text with no clear indication that LLM provider was disabled/fallback mode.
- **Root cause:** UI rendered `quick_actions` as passive labels (`span`) without click handlers. Chat send path used `/agent/message` (message-only response), which dropped provider metadata (`used_fallback`), so users could not tell if real LLM was used.
- **Fix:** Quick actions are now clickable buttons wired to real actions (`scan_now` -> scan endpoint; comparison/cross-pair actions -> send prefilled prompt). Chat send in `Position Agent` now uses `/agent/llm-reply` and surfaces reply source (`fallback/provider:model`) in UI info message.
- **Guards/tests:** `npx tsc --noEmit` in `web/`.
- **Paths:** `web/src/pages/PositionDetail.tsx`, `web/src/lib/api.ts`

---

### BUG-20260421-01 — Backtests FULL: duplicate strategy labels hid different ranges; global metrics were unclear

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-21  
fixed_in: local  
keywords: backtests, full-run, optimize, duplicates, strategy-range, meteora, lp_share, tooltips, ranking

- **Symptom:** TOP list for a pool/window could show the same strategy label multiple times (e.g. `bollinger_w20_k2.5_r48` twice) with slightly different metrics and no visible reason.
- **Symptom:** Global ranking column `Wystapienia` looked suspicious (e.g. `320`) and users interpreted it as wins instead of count of tested variants.
- **Symptom:** Meteora rows frequently showed very small fees (e.g. `0.03`) because API silently forced `--lp-share 0.0001` when request had no `lp_share`.
- **Root cause:** API returned only strategy label + metrics parsed from CLI table and dropped per-row range context (`Lower($)`, `Upper($)`), while UI TOP3 selected first 3 rows after sort (not unique strategy labels). Global table lacked metric semantics hints. Meteora fallback share was hardcoded in API handler.
- **Fix:** Added range context to API metric rows (`lower_usd`, `upper_usd`, `width_pct`), made TOP3 unique by strategy label (best variant per label for selected sort), displayed range/width in per-window table, added column/tooltips clarifying semantics (including `Wystapienia`), and removed forced Meteora fallback `lp_share=0.0001` in API.
- **Guards/tests:** `npx tsc --noEmit` in `web/`; `cargo check -p clmm-lp-api`.
- **Paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/backtests.rs`, `web/src/lib/api.ts`, `web/src/pages/Backtests.tsx`

---

### BUG-20260420-04 — `backtest-optimize` skipped documented strategies (`oor_recenter`, `il_limit`, `retouch_shift`)

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-20  
fixed_in: local  
keywords: backtest-optimize, strategies, StratConfig, oor_recenter, il_limit, retouch_shift, docs-drift

- **Symptom:** `backtest-optimize` results did not include all strategies described in docs, and rankings were biased toward `static` / indicator variants because `oor_recenter`, `il_limit`, `retouch_shift` were missing from the grid.
- **Root cause:** `StratConfig` + `default_strategies()` in CLI omitted those strategy variants, while documentation still described them as part of optimize strategy catalog.
- **Fix:** Added `OorRecenter`, `IlLimit`, `RetouchShift` to `StratConfig`, wired trigger/range logic in `run_single`, added parser support (`parse_strategy_label`), and restored those variants in `default_strategies()` (using `--il-max-pct`, `--il-close-pct`, `--il-grace-steps`).
- **Guards/tests:** Added parser regression assertions for new labels and a grid regression test ensuring default optimize set includes the missing strategies; verified with `cargo test -p clmm-lp-cli parse_strategy_label_bollinger_and_last_candle` and `cargo test -p clmm-lp-cli default_grid_includes_documented_non_indicator_strategies`.
- **Paths:** `crates/cli/src/backtest_engine.rs`, `crates/cli/src/commands/backtest_optimize.rs`, `crates/cli/src/output/optimize_result_json.rs`, `crates/cli/src/engine/tests.rs`, `doc/BACKTEST_OPTIMIZE_STRATEGIES.md`

---

### BUG-20260420-03 — Stream PnL mixed chain-anchored valuation with component-wide session costs

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-20  
fixed_in: local  
keywords: stream-pnl, stream-lineage, bfs-component, rebalance-session-id, tx-fees, realized-cashflow, forked-edges

- **Symptom:** Stream totals could show internally inconsistent scope: `baseline/current` anchored to the selected lineage chain (`start -> ... -> end`), while `tx_fees_usd` and `realized_cashflow_usd` could include rows from sibling branches in the same BFS component.
- **Symptom (example):** In forked graph shape (`A->B->C` plus `A->X`), querying chain ending at `C` could still include `A->X` session fees/cashflow in totals.
- **Root cause:** `compute_position_stream_pnl_for_stream_members` used `sessions` derived from full component connectivity (`compute_position_stream_performance`) for cost/cashflow aggregation, but valuation anchors were lineage-specific (first/last chain PDA).
- **Fix:** Added chain-local session scoping in `position_stream_pnl`: derive `chain_sessions` from DB edges matching adjacent ordered chain pairs only; aggregate tx fees and token-delta cashflow by those sessions. If chain sessions are unavailable, fallback to chain positions (not component-wide sessions).
- **Guards/tests:** Added unit tests `chain_sessions_ignore_fork_edges_outside_ordered_chain` and `chain_sessions_empty_for_single_node_chain`; verified with `cargo test -p clmm-lp-api chain_sessions_`.
- **Paths:** `crates/api/src/services/position_stream_pnl.rs`

---

### BUG-20260420-02 — `experiment-config` failed to derive USD capital when pool only existed in lifecycle rows

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-20  
fixed_in: local  
keywords: experiment-config, derived_initial_capital_usd, pool_address, pool_pubkey, open-session, positions-handler

- **Symptom:** `Detected config (open snapshot)` showed `Could not resolve pool mints to convert open-session deltas to USD; derived_initial_capital_usd unavailable.` even when lifecycle/session data existed.
- **Root cause:** `get_position_experiment_config` resolved pool mints only from `registry_open.details.pool_address`; older/incomplete rows often had pool only in lifecycle ledger lines (`pool_address`/`pool_pubkey`) for the same `rebalance_session_id`.
- **Fix:** Added fallback pool resolution for the open session by scanning lifecycle rows for the same session id and taking `pool_pubkey` or `pool_address` when details are missing.
- **Guards/tests:** `cargo build -p clmm-lp-api`.
- **Paths:** `crates/api/src/handlers/positions.rs`

---

### BUG-20260420-01 — Position Detail totals lost/zeroed chain-level LP collected summary

status: partially fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-20  
fixed_in: local  
keywords: position-detail, stream-lineage, chain_cost_summary, fees_collected_usd_total, ui-regression, lifecycle-ingest, pool_address, pool_pubkey

- **Symptom:** On `PositionDetail` (`Logs / rebalances`), the totals card still showed baseline/current/tx fees/cashflow/net PnL, but chain-level **LP collected** summary disappeared after the economic-vs-IL layout update.
- **Symptom (follow-up):** After restoring the field, some closed chains still showed zeros for tx/LP collected although lifecycle JSONL rows existed (`bot_collect_fees`, non-zero `tx_fee_lamports`).
- **Symptom (follow-up 2):** In `Closed position` view, chain LP fee leg rows showed only token units/base units (e.g. `SOL`, `whETH`) without per-leg USD approximation, making it hard to reconcile with summed USD.
- **Root cause:** (1) UI refactor replaced the old totals block and omitted the `chain_cost_summary.fees_collected_usd_total` + `collect_events_total` rendering branch. (2) DB ingest/aggregation path was brittle for lifecycle rows using `pool_address` (not `pool_pubkey`) and DB fallback triggered only when **all** node values were empty, so partial DB rows could still zero tx/collect aggregates.
- **Fix:** Restored `LP collected (sum)` in UI totals; lifecycle ingest now accepts `pool_address` as fallback for `pool_pubkey`; DB node metrics now bridge tx/collect aggregates from lifecycle JSONL when DB returns zeros for those fields.
- **Fix (follow-up 2):** Added per-leg USD approximation in chain fee leg rows (`≈ $...`) using current mint prices from Jupiter for displayed token mints; keeps `—` when mint price is unavailable.
- **Guards/tests:** `npx tsc --noEmit` in `web/`; `cargo test -p clmm-lp-api position_stream_lineage`.
- **Paths:** `web/src/pages/PositionDetail.tsx`, `crates/api/src/services/position_stream_performance.rs`, `crates/api/src/services/position_stream_lineage.rs`

---

### BUG-20260419-01 — Stream PnL IL/HODL silently wrong: valuation snapshot queries omitted mint columns

status: fixed  
severity: high  
reported_by: ai  
first_seen: 2026-04-19  
fixed_in: local  
keywords: stream-pnl, position_stream_pnl, hodl, il_usd, valuation_snapshots, token_mint_a, sql

- **Symptom:** With rows in `position_stream_valuation_snapshots` that already stored `token_mint_a` / `token_mint_b`, `/positions/.../stream-lineage` totals still behaved like “IL unavailable” semantics: `hodl_value_usd` fell back to `baseline_value_usd`, so `il_usd` became `current_value − baseline_value` instead of baseline basket × current mint prices − LP mark.
- **Root cause:** `compute_position_stream_pnl_for_stream_members` selected only `ts_utc, value_usd, amount_a_ui, amount_b_ui, pool_pubkey` but later read `token_mint_*` from the row — columns were never fetched, so mint paths were always empty.
- **Fix:** Extended baseline and latest snapshot queries to include `token_mint_a`, `token_mint_b`; resolve pool mints with baseline-first + per-leg fallback to the latest snapshot; annotate `ts_utc` row decode for inference.
- **Guards/tests:** Unit tests `pool_mints_prefers_*`, `pool_mints_falls_back_*`, `pool_mints_mixed_fallback_per_leg` in `services::position_stream_pnl`.
- **Paths:** `crates/api/src/services/position_stream_pnl.rs`

---

### BUG-20260417-02 — Backtests FULL run: `unexpected argument '--include-strategy-families'` (stale CLI)

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-17  
fixed_in: local  
keywords: backtests, backtest-optimize, include-strategy-families, clmm-lp-cli, api, resolve_clmm_lp_cli_path

- **Symptom:** Web **Backtests** → FULL comparison failed with `exit Some(2)` and stderr: `unexpected argument '--include-strategy-families' found` (CLI suggested `--indicator-strategies`).
- **Root cause:** API spawned an **older** `clmm-lp-cli` binary (commonly `target/debug/clmm-lp-cli.exe` next to `cargo run` API) that predates `--include-strategy-families`, while the API always passed that flag for strategy-family filtering.
- **Fix:** `resolve_clmm_lp_cli_path` now prefers **`target/release` before `target/debug`** under `CLMM_REPO_ROOT` (and same order for `CLMM_API_TARGET_DIR` / `CARGO_TARGET_DIR`) and only falls back to “next to API exe” after those candidates. Full-matrix handler probes `backtest-optimize --help`; omits the flag for **full-catalog** runs on legacy CLI, and **fails fast** with a clear rebuild message when a **subset** of strategies is selected but the CLI cannot filter families.
- **Fix (follow-up):** Probe result cache key includes the CLI binary **mtime**, so rebuilding `clmm-lp-cli` invalidates stale “unsupported” results without restarting `clmm-lp-api`.
- **Paths:** `crates/api/src/handlers/backtests.rs`

---

### BUG-20260417-01 — `data_alerts_loop.ps1` fails on empty `$PSScriptRoot`

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-17  
fixed_in: local  
keywords: windows, powershell, data_alerts_loop, PSScriptRoot, RepoRoot, Join-Path

- **Symptom:** Running `powershell -File .\tools\data_alerts_loop.ps1` failed immediately with `Join-Path : Cannot bind argument to parameter 'Path' because it is an empty string.` at param default for `RepoRoot`.
- **Root cause:** `RepoRoot` default evaluated `Join-Path $PSScriptRoot ".."` during parameter binding; in some hosts/sessions `$PSScriptRoot` was empty, so binding failed before script logic started.
- **Fix:** Changed `RepoRoot` default to empty and resolved it at runtime with fallbacks: `$PSScriptRoot` -> `Split-Path $MyInvocation.MyCommand.Path` -> `Get-Location`.
- **Symptom (follow-up):** The same path-empty failure appeared in `tools/snapshot_health_alert.ps1` (and could affect scheduled-task registration helper), because they used the same default-parameter pattern.
- **Fix (follow-up):** Applied the same runtime `RepoRoot` resolution hardening in `tools/snapshot_health_alert.ps1` and `tools/register_snapshot_health_scheduled_task.ps1`.
- **Guards/tests:** Re-ran `tools/data_alerts_loop.ps1` with explicit minimal intervals and `-SkipSlack` to verify startup no longer fails at parameter binding.
- **Guards/tests (follow-up):** Re-ran `powershell -File .\tools\snapshot_health_alert.ps1 -SkipSlack` and confirmed it reaches health-check execution instead of failing in parameter binding.
- **Paths:** `tools/data_alerts_loop.ps1`, `tools/snapshot_health_alert.ps1`, `tools/register_snapshot_health_scheduled_task.ps1`

---

### BUG-20260416-02 — Snapshot loops stopped collecting when release CLI binary was missing

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-16  
fixed_in: local  
keywords: snapshots, collector-loop, run-snapshot-loop, run-snapshot-loop-5m, clmm-lp-cli, release-binary, cargo-fallback

- **Symptom:** Snapshot freshness checks showed `rows_in_window~=0` for both `snapshots.jsonl` (10m) and `snapshots_5m.jsonl` (5m), while loop logs repeatedly reported `target\\release\\clmm-lp-cli.exe` not found.
- **Symptom (follow-up):** In service-like loop context, `cargo` was not in PATH, so the first fallback attempt also failed with `The term 'cargo' is not recognized...`.
- **Root cause:** Windows loop scripts hard-required `target/<configuration>/clmm-lp-cli.exe` and only logged errors when absent; no runtime fallback existed, so periodic collection silently stopped.
- **Fix:** Added runtime fallback in both loop scripts: when release binary is missing, execute collector via `cargo run -q -p clmm-lp-cli --bin clmm-lp-cli -- snapshot-run-curated-all` (and `--snapshots-suffix 5m` for 5m loop). Logs now explicitly record fallback mode on startup.
- **Fix (follow-up):** Added cargo path resolver (`CARGO_HOME`, `%USERPROFILE%\\.cargo\\bin\\cargo.exe`, then PATH lookup) so fallback works under service contexts where PATH is minimal.
- **Guards/tests:** Re-ran collector health scripts to confirm root cause and verify that loops have a valid execution path even without release artifact.
- **Guards/tests (follow-up):** Loop heartbeats (`data/snapshot_logs/snapshot-loop-heartbeat-{10m,5m}.json`) + `snapshot_health_check.ps1` stale-heartbeat issues; Windows: `tools/register_snapshot_health_scheduled_task.ps1` lub `tools/data_alerts_loop.ps1` pod Shawl/NSSM — automatyczne `snapshot_health_alert` bez ręcznego sprawdzania.
- **Paths:** `scripts/windows/run-snapshot-loop.ps1`, `scripts/windows/run-snapshot-loop-5m.ps1`, `tools/snapshot_health_check.ps1`, `tools/register_snapshot_health_scheduled_task.ps1`

---

### BUG-20260415-01 — Position Detail vs Positions list: inconsistent strategy-link badge

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-15  
fixed_in: local  
keywords: positions-ui, position-detail, monitored-positions, linked-strategy, react-query, stale-cache

- **Symptom:** Dla tej samej pozycji (`/positions/<PDA>`) w `Position Info` strategia była widoczna jako podłączona, ale na stronie `Positions` w tabeli `Monitored positions (API)` pojawiało się `Not linked`.
- **Root cause:** Widok `Positions` opierał się na mapowaniu `strategies -> position_addresses` bez wymuszonego refetch na mount oraz bez normalizacji klucza po stronie pozycji z monitora. Przy cache / odświeżeniach między podstronami mogło to dawać rozjazd statusu.
- **Fix:** W obu widokach (`PositionDetail`, `Positions`) zapytanie `['strategies']` wymusza odświeżenie na mount. W `Positions` dodano normalizację adresu (`trim`) po obu stronach mapowania (`position_addresses` i `position.address`) przed lookupem.
- **Fix (follow-up):** Dla `['strategies']` w tych widokach ustawiono też `staleTime: 0`, `refetchOnWindowFocus: true` i krótki `refetchInterval` (15s), aby zbić ryzyko utrzymywania starego link-statusu przy długiej sesji SPA i globalnym cache `staleTime=5m`.
- **Fix (follow-up 2):** `PositionDetail` filtruje listę linked strategii przez `diagnostics.linked_strategies` (backend source-of-truth), więc sekcja `Position Info` nie pokazuje linku, którego backend już nie widzi.
- **Fix (follow-up 3):** `Positions` (`Monitored positions (API)`) również opiera badge `Strategy` o `position-diagnostics` per-wiersz (a nie tylko `GET /strategies`), więc status `linked/not linked` jest zgodny z backend source-of-truth także na liście.
- **Symptom (2026-04-27, regression):** Dla pozycji z linked strategią sekcja `PositionDetail -> Diagnostics` pokazywała strategię poprawnie, ale `Positions -> Monitored positions (API)` wracało do `Not linked`.
- **Root cause (2026-04-27, regression):** W `Positions` status był nadal liczony jako przecięcie dwóch źródeł (`diagnostics.linked_strategies` AND `GET /strategies` mapowane po `position_addresses`). Gdy diagnostics miało link, a cache/config `/strategies` chwilowo nie zawierało tego PDA, UI pokazywał false-negative `Not linked`.
- **Fix (follow-up 4):** `Positions` renderuje status linku bezpośrednio z `position-diagnostics` (source-of-truth); `GET /strategies` służy tylko do wzbogacenia opisu parametrami, z fallbackiem do danych z diagnostics gdy szczegóły strategii nie są dostępne.
- **Symptom (2026-04-29, regression):** Po pending-open recovery nowy aktywny PDA potrafił pozostać `Not linked`, mimo że strategia była running; jednocześnie stary close/open session mógł dalej wisieć jako pending/stranded.
- **Root cause (2026-04-29, regression):** Pending-open queue jest globalna i item może zostać obsłużony przez dowolny executor. `reopen_hook` aktualizował wcześniej tylko `strategy_id` executora, który akurat podniósł item, zamiast strategii faktycznie trzymającej `old_position` w `position_addresses`.
- **Fix (follow-up 5):** `reopen_hook` rozwiązuje teraz właścicieli po `old_position` (`strategy_ids_holding_position_address`) i wykonuje replace `old -> new` dla każdej pasującej strategii, a następnie synchronizuje managed allowlist tych strategii. Fallback do pierwotnego `strategy_id` zostaje tylko gdy nie znaleziono właściciela.
- **Guards/tests:** Weryfikacja ręczna na tej samej pozycji: `Position Info` i `Monitored positions (API)` pokazują spójny status linku.
- **Paths:** `web/src/pages/Positions.tsx`, `web/src/pages/PositionDetail.tsx`, `crates/api/src/services/strategy_service.rs`

---

### BUG-20260414-08 — New position detail showed full prior rotation “history” (merged stream component)

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: stream-lineage, position_stream_edges, suppress_jsonl_rotation_stitch, compute_position_stream_lineage, fresh-open, BFS

- **Symptom:** After closing an old NFT and opening a new one, Position detail’s lineage/history table listed many unrelated prior PDAs (same pool) as if they belonged to the new mint.
- **Root cause:** JSONL/registry fallback respected `suppress_jsonl_rotation_stitch` (manual / unanchored opens), but the **DB path** always queried edges using the full undirected BFS component from `compute_position_stream_performance`, which can merge unrelated PDAs when the edge graph is noisy.
- **Root cause (follow-up):** `lifecycle_rotation_parent_before_open` treated **`close_kind=rotation` alone** on an older `bot_close_position` as rotation evidence. API opens are logged as **`bot_open_position`** with a **new** `rebalance_session_id` (`cost_session_id`); a recent unrelated rotation close in the same pool/payer window became a false “parent”, so `suppress_jsonl_rotation_stitch` stayed false and the long chain reappeared even after the DB isolation fix.
- **Fix:** When `suppress_jsonl_rotation_stitch` is true, DB lineage uses an **entry-only** stream member list for the edge query + `build_lineage_chain_from_db_edges`, and stream PnL totals use the same restriction via `compute_position_stream_pnl_for_stream_members`. **Parent inference** no longer uses `close_kind=rotation` without a **session id match** (close vs open) or **bot-tied** rows on the closed PDA in the pre-open window.
- **Fix (follow-up 2):** **Operator** semantics in lifecycle JSONL: `position_open` / `source:cli` / `details.open_origin=operator_api` ⇒ **always** suppress prior history; API open writes `open_origin`; **manual** closes (`position_close`, `close_kind=manual`, `close_source=api`) do not act as rotation parents and stop **forward** JSONL chain walks (commit `4836a2c` was the earlier “no false history” baseline).
- **Guards/tests:** `db_edges_entry_only_positions_ignore_external_rotation_neighbor`, `rotation_parent_ignores_ambient_close_kind_rotation_without_session_or_bot_tie`; `cargo test -p clmm-lp-api services::position_stream_lineage::tests`.
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`, `crates/api/src/services/position_stream_pnl.rs`

### BUG-20260414-07 — Manual “Close position” reported success but sent no on-chain close (dry-run strategy executor)

status: fixed  
severity: critical  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: close-position, resolve_executor_for_position_ops, dry_run, StrategyExecutor, position ops

- **Symptom:** Operator clicked Close multiple times; UI/API returned success (“position closed”) but the Whirlpool position remained open on-chain.
- **Root cause:** `resolve_executor_for_position_ops` returned the **first** strategy executor in the map. If that strategy was `dry_run=true`, `RebalanceExecutor::execute_full_close_only` **no-oped** (returns `Ok(())` without submitting txs), while `PositionService` still returned `OperationResult::success()`.
- **Fix:** Resolve only executors with `!is_dry_run()` **and** `wallet_pubkey().is_some()`; prefer `__api_position_ops__`, then any qualifying strategy runner, then create the lazy ops executor from env keypair. Added `StrategyExecutor::is_dry_run()`.
- **Guards/tests:** `cargo build -p clmm-lp-api`.
- **Paths:** `crates/api/src/services/position_executor.rs`, `crates/execution/src/strategy/executor.rs`

### BUG-20260414-06 — `min_rebalance_interval_hours: 0` caused a close+open every eval tick (~5m) while in-range

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: periodic, min_rebalance_interval_hours, eval_interval_secs, rebalance-loop, strategy-parameters

- **Symptom:** Multiple rebalances within ~25 minutes (e.g. 5×) while the position stayed in range; cadence matched **`eval_interval_secs`** (default 300s), not “every N hours”.
- **Root cause:** `DecisionConfig` ties Periodic to `hours_since_rebalance >= periodic_interval_hours`. When **`min_rebalance_interval_hours` / periodic interval is `0`**, `hours_since >= 0` is always true → **Rebalance on every executor tick**. UI could send `0` via numeric field; persisted JSON could also contain `0`.
- **Fix:** Follow-up policy split: `periodic` still guards against `0` (frontend blocks `0`, backend clamps `0 -> 1` defensively if it arrives via API), while non-periodic strategies accept `0` as “no time gate”. Optional empty interval now stays optional (no implicit 1h/24h clamp in strategy parameter mapping).
- **Guards/tests:** `min_rebalance_interval_parses_json_number_and_string`; UI validation for `periodic` rejects `0` with explicit message.
- **Paths:** `crates/api/src/services/strategy_service.rs`, `crates/api/src/handlers/strategies.rs`, `web/src/lib/strategyFormShared.tsx`

### BUG-20260414-05 — Strategy UI stuck on first linked PDA after bot rotations; HTTP start / autostart used divergent executor wiring

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: strategies, reopen-hook, position_addresses, start-strategy, autostart, StrategyService, state.executors, periodic-rebalance

- **Symptom:** Bot closed and reopened positions (new NFT PDAs) while price stayed in range; dashboard strategy still showed the **original** `parameters.position_addresses` entry (e.g. first mint), not the active position.
- **Root cause (hook):** `start_strategy_executor_core` (used by `POST /strategies/{id}/start`, `ensure_executor_after_link`, and `PUT /strategies/{id}` restart) did **not** register `set_reopen_hook` or `set_managed_allowlist`, so close→open cycles did not call `replace_position_address_in_strategy`.
- **Root cause (map):** `StrategyService::start_strategy` (API boot autostart) stored executors in a **private** `HashMap` separate from `AppState.executors`, so stop/sync/heal paths that read `state.executors` could not see autostarted runners (and behavior diverged from HTTP-started strategies).
- **Fix:** Introduced shared `wire_executor_allowlist_and_reopen_hook`; `start_strategy_executor_core` calls it. `StrategyService` now uses `state.executors` only (removed duplicate map).
- **Guards/tests:** `cargo build -p clmm-lp-api`; existing `managed_allowlist_*` tests still pass.
- **Paths:** `crates/api/src/services/strategy_service.rs`, `crates/api/src/handlers/strategies.rs`

### BUG-20260414-04 — Clearing `position_addresses` via strategy update did not stick; empty list widened executor scope

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: strategies, put-strategies, position_addresses, linked-positions, update_strategy, managed-allowlist, executor

- **Symptom:** After editing `data/strategies.json` or calling `PUT /strategies/{id}` with `position_addresses: []`, the dashboard still showed long **Linked positions** lists; operators could not reset strategy links for a clean test cycle.
- **Root cause (persistence):** `update_strategy` always re-inserted the previous `position_addresses` JSON into the new config, so an explicit clear in the request body was overwritten.
- **Root cause (semantics):** When `position_addresses` was an empty array, `start_strategy` / `sync_managed_allowlist_from_registry_for_strategy` treated “no configured PDAs” as “use all registry-open PDAs”, which could make a “cleared” strategy still drive automation across unrelated positions after autostart.
- **Fix:** `update_strategy` only restores legacy `position_addresses` / `executor_disabled_position_addresses` when the request omits those fields (`None`); if the client sends them (including `[]`), the request wins. Centralized allowlist helper: explicit `[]` ⇒ empty managed set; missing or non-array field keeps legacy “registry-open” fallback.
- **Guards/tests:** Unit tests `missing_position_addresses_field_uses_registry_open`, `explicit_empty_position_addresses_yields_empty_allowlist`.
- **Paths:** `crates/api/src/handlers/strategies.rs`, `crates/api/src/services/strategy_service.rs`

### BUG-20260414-03 — Stranded list had no operator dismiss control

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: stranded-rebalances, pending-open-recovery, watchdog, ui-action, dismiss-session

- **Symptom:** `Closed by bot, waiting for reopen` accumulates stale/noisy entries with no way to remove them from UI. User could not prepare clean test runs and wanted removed sessions to stop influencing bot recovery flow.
- **Symptom (follow-up):** Removed sessions returned after some time.
- **Symptom (follow-up 2):** `dismissed_session_ids` remained empty in runtime `pending-open-recovery.json` despite UI `Remove`.
- **Symptom (follow-up 3):** After removing one stranded row, bot could still auto-open from another queued pending item with the same `pool + intended range`, so list clearing did not fully stop reopen attempts during manual tests.
- **Symptom (follow-up 4):** `pending-open-recovery.json` still contained reopen items while `Closed by bot, waiting for reopen` section was empty, so operator could not remove hidden queue entries from UI.
- **Symptom (follow-up 5):** Even after removing visible `pending:*` rows, entries could reappear from another queued/derived item in the same `pool + intended range` group (different old position/session id), so operator cleanup still felt non-deterministic.
- **Root cause:** Watchdog/API exposed only read/reconcile operations (`get`/`reconcile`), with no persistent denylist/dismiss mechanism. Pending-open queue could continue using previously queued rows.
- **Root cause (follow-up):** Execution-side pending-open store schema lacked `dismissed_session_ids`; when bot wrote pending-open file, dismiss metadata was dropped, so sessions reappeared.
- **Root cause (follow-up 2):** Dismiss persistence depended on one shared JSON file; if another process rewrote the file without dismiss metadata, hidden sessions resurfaced.
- **Fix:** Added persistent session dismiss flow: `POST /bot-activity/stranded-rebalances/{session_id}/dismiss`. Dismissed session ids are stored in pending-open JSON, excluded from stranded snapshot/reconcile, and matching pending-open item for that session's old position is removed. Execution pending-open schema now preserves `dismissed_session_ids` on load/save.
- **Fix (follow-up 2):** Added separate persisted denylist file for dismissed sessions (`data/stranded-dismissed-sessions.json`, env override `CLMM_STRANDED_DISMISSED_PATH`) and merged it into snapshot/reconcile filters.
- **Fix (follow-up 3):** Dismiss now prunes pending-open queue by both exact `closed_position_nft` and by `pool + intended_tick_lower + intended_tick_upper` group, so operator cleanup keeps stranded/pending views coherent and blocks same-range auto-reopen leftovers.
- **Fix (follow-up 4):** `stranded-rebalances` snapshot now includes synthetic `pending-only` rows for queued reopen items that have no visible lifecycle close row, so every reopen-capable queue item is visible/removable from UI.
- **Fix (follow-up 5):** Dismiss now stores an additional denylist marker per `pool + intended range` (`pending-group:<pool>:<lower>:<upper>`). Snapshot/hide and reconcile/auto-enqueue both honor this group marker, preventing reappearance from sibling sessions/items in the same reopen group.
- **Guards/tests:** Added regression test `dismissed_session_is_excluded_from_stranded_list`; watchdog suite passes.
- **Guards/tests:** Added regression test `pending_open_store_parses_and_keeps_dismissed_sessions`.
- **Guards/tests (follow-up 3):** Added regression test `dismiss_prunes_pending_by_old_position_and_pool_range`.
- **Guards/tests (follow-up 4):** Added regression test `pending_only_item_is_visible_in_stranded_output`.
- **Guards/tests (follow-up 5):** Added regression test `dismissed_pending_group_hides_all_matching_pending_rows`.
- **Paths:** `crates/api/src/services/stranded_rebalance_watchdog.rs`, `crates/api/src/handlers/bot_activity.rs`, `crates/api/src/routes.rs`, `web/src/pages/Positions.tsx`, `web/src/lib/api.ts`, `crates/execution/src/strategy/pending_open.rs`

### BUG-20260414-02 — Manual close appeared in “Closed by bot, waiting for reopen”

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: stranded-rebalances, manual-close, bot-close, close-kind, close-source, ui-section

- **Symptom:** Po ręcznym zamknięciu pozycji wpis potrafił pojawić się w sekcji `Closed by bot, waiting for reopen`, co sugerowało nieprawidłową klasyfikację.
- **Root cause:** Watchdog budujący `stranded-rebalances` traktował każdy `event=bot_close_position` jako close botowy, bez sprawdzenia `details.close_kind` / `details.close_source`.
- **Fix:** Dodano filtr: rekordy close z `details.close_kind=manual` lub `details.close_source=api` są wykluczane z listy stranded.
- **Guards/tests:** Dodany test `manual_close_event_is_excluded_from_stranded_list`.
- **Paths:** `crates/api/src/services/stranded_rebalance_watchdog.rs`

### BUG-20260414-01 — Manual close returns opaque Whirlpool custom 3007

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-14  
fixed_in: local  
keywords: close-position, whirlpool, custom-3007, account-owned-by-wrong-program, signer-wallet, position-nft

- **Symptom:** Manual close failed with `InstructionError(2, Custom(3007))` on Whirlpool ix (`whirLb...`) and API returned generic bad-request text without actionable ownership hint.
- **Root cause:** `classify_close_position_error` handled `6018`/slippage but had no dedicated branch for Whirlpool `3007`, so operators could not distinguish account-ownership mismatch from slippage/funding issues.
- **Fix:** Added explicit close error mapping for `custom 3007` (`custom(3007)` / `custom_code=3007`) with clear hint: verify API signer owns required position/token accounts (especially position NFT ownership).
- **Fix (follow-up):** `close_position` path now treats `3007` as idempotent success **when registry already marks the PDA as closed** (removes stale monitor entry instead of surfacing repeated failure).
- **Guards/tests:** Added regression test `close_position_error_3007_maps_to_bad_request_with_account_hint`.
- **Paths:** `crates/api/src/services/position_service.rs`

### BUG-20260413-06 — Stan portfela na „nowa pozycja” bywa nieaktualny / trzeba odświeżyć

status: open  
severity: medium  
reported_by: user  
first_seen: 2026-04-13  
fixed_in:   
keywords: position-create, wallet-balances, api-signer, stale-data, react-query, race, owner-pubkey

- **Symptom:** Przy wejściu na flow otwarcia nowej pozycji odczyt „Stan portfela” bywa niepełny lub wygląda na opóźniony względem tego, co widać na stronie Wallet; czasem dopiero pełne odświeżenie strony pokazuje oczekiwane salda.
- **Root cause (analiza kodu):** W `PositionCreate` `effectiveOwnerPk` jest liczone jako `apiSigner.pubkey ?? ownerPk` (`ownerPk` = wybrany portfel z listy keypairów w localStorage). Dopóki zapytanie `GET /wallets/api-signer` się nie zakończy, używane jest **tymczasowo** `ownerPk`, więc `useQuery(['wallet-balances', effectiveOwnerPk])` może najpierw pobrać salda **innego** klucza niż ten, którym realnie podpisuje open (API signer). Po dojściu odpowiedzi api-signer klucz query się zmienia i następuje drugi fetch — użytkownik może złapać „złą” chwilę UI lub porównywać z Wallet (tam **zawsze** `ownerPk`, bez api-signer). Dodatkowo `staleTime: 20_000` i globalne `refetchOnWindowFocus: false` sprzyjają pokazywaniu cache bez natychmiastowego ponownego odczytu przy nawigacji.
- **Fix:** Do wdrożenia: np. nie włączać `wallet-balances` na PositionCreate dopóki `api-signer` nie jest `isFetched` (gdy signer jest skonfigurowany), albo jedno zapytanie łączone; ewent. `refetchOnMount: 'always'` / krótszy `staleTime` tylko dla tej strony; jasny komunikat „ładowanie portfela podpisującego…” zamiast chwilowych sald z niewłaściwego ownera.
- **Guards/tests:** Test E2E lub jednostkowy: kolejność rozwiązań query (wallets vs api-signer) nie powinna wyświetlać sald dla złego pubkey; regresja: po swap/open invalidate trafia w ten sam `effectiveOwnerPk`.
- **Paths:** `web/src/pages/PositionCreate.tsx` (`effectiveOwnerPk`, `apiSignerQ`, `effectiveBalancesQ`), `web/src/pages/Wallet.tsx` (porównanie: samo `ownerPk`), `web/src/main.tsx` (domyślne opcje React Query), `crates/api/src/handlers/wallets.rs` (`GET /wallets/api-signer`)

### BUG-20260413-07 — Performance „Value” vs Position history (start/end): duża rozbieżność (~2×) na otwartej pozycji

status: partially fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: position-detail, performance, stream-lineage, value-usd, snapshots, valuation-drift, UI-consistency

- **Symptom:** Na `/positions/<PDA>` (otwarta pozycja) karta **Performance → Value** pokazuje np. **~$3.88**, a w **Position history (rotations)** kolumny **start value** i **end value** obie np. **~$1.80** z etykietą `exact` — użytkownik widzi pozornie „dwukrotnie mniej” w tabeli niż w Performance.
- **Symptom (2026-04-14):** Karta **Performance → Value** bywa „historyczna”/zaniżona po wejściu na detal pozycji (frontend cache), mimo że oczekiwana jest bieżąca wycena.
- **Symptom (2026-04-14, follow-up):** Po wdrożeniu odświeżania na mount, w części sesji `Performance -> Value` nadal wygląda na zaniżone (np. ~$2.278) względem oczekiwanej bieżącej wyceny.
- **Symptom (2026-04-14, UI semantics):** Użytkownik mieszał „live value” (single PDA) z agregatem historycznym streamu; etykieta `end value` dla open node była czytana jako „przyszła wartość”.
- **Symptom (2026-04-14, follow-up):** Świeżo otwarta pozycja mogła mieć `opened == closed/last` i widoczne `end value` już na starcie, mimo braku close.
- **Symptom (2026-04-14, runtime chain jump):** Dla jednego chainu (SOL/USDC) kolejne node’y pokazały sekwencję `~2.01 -> ~0.98 -> ~3.05 -> ~6.00` przy niewielkiej zmianie ceny rynkowej; użytkownik raportuje efekt „uciętej nogi” w jednym kroku i „dodanej nogi” w następnym. To wygląda jak niespójna wycena per-node (nieadekwatna do realnego market move), a nie zwykły wynik ceny.
- **Symptom (2026-04-14, close accounting):** `end value` dla części zamknięć bywało policzone tylko z jednej nogi puli, bo `fee_payer_token_deltas` z tx meta nie zawsze zawiera obie nogi (typowo brak jednej nogi przy WSOL/ATA flow).
- **Symptom (2026-04-15, recovery follow-up):** Po `rebalance_incomplete` i recovery-open, nowe `baseline_open` potrafiło być zawyżone (np. ~7 vs oczekiwane ~3) przez mixed-sign delty na open.
- **Root cause (analiza kodu):** Ta metryka „wartość USD” jest liczona w **dwóch miejscach** (nie jest jednym cache): karta Performance vs wiersz lineage. Karta Performance bierze `value_usd` z `GET /positions/{address}`: świeże `compute_position_usd_valuation` na stanie z monitora/RPC (`handlers/positions.rs`). Tabela lineage w `GET /positions/{address}/stream-lineage` dla węzła w ścieżce DB w `node_metrics` ustawia **baseline** i **current** z tabeli `position_stream_valuation_snapshots` (pierwszy vs ostatni wiersz wg sortowania SQL), a niekoniecznie z tą samą chwilą ani tą samą logiką co bieżąca karta. Dla **świeżo otwartej** pozycji często jest **jeden** (lub kilka) snapshotów — **start i end mogą wskazywać ten sam zapis**, z wartością zapisaną wcześniej (inny moment/ceny/jedna noga w delcie) podczas gdy karta już pokazuje nowszą wycenę. Etykieta `exact` w UI pochodzi z `raw_json.valuation_quality` snapshotu lub z ścieżki lifecycle — opisuje **jakość wejścia cen**, nie gwarancję zgodności z kartą Performance.
- **Fix:** (2026-04-13, uproszczenie) Zamiast wielu heurystyk w `node_metrics`, **`persist_event_valuation_snapshots_for_positions`** przy zapisie **`baseline_open`** uzupełnia **tylko brakującą nogę** (delta `amount_*_ui == 0`) z **`details.amount_*_cap`** (Orca max raw); druga noga zostaje z delt. **Pełne zastąpienie obu nóg capami** usunięte — cap to maksimum, nie faktyczny depozyt; dawało zawyżenie wobec **Performance** (np. ~$6.15 vs ~$5.60). `baseline_amounts_source: "open_caps"`. **`ON CONFLICT DO UPDATE`** tylko przy `open_caps`. `node_metrics` — prosty odczyt snapshotów + istniejący guardrail z ledgera.
- **Fix (2026-04-14):** `PositionDetail` wymusza świeży fetch `GET /positions/{address}` na mount (`staleTime: 0`, `refetchOnMount: 'always'`). API zwraca też `valuation_source` (`live_valuation` vs `fallback_monitor`) dla `value_usd`, więc UI odróżnia świeżą wycenę od fallbacku monitora.
- **Fix (2026-04-14, UI):** `PositionDetail` rozdziela semantykę sekcji: `Live value (this position, now)` + jawny opis źródła dla single-PDA endpointu; stream dostał nagłówek „history summary across rotated PDAs”; kolumna historii `end value` zmieniona na `current/end value`.
- **Fix (2026-04-14, follow-up):** W DB-path lineage `closed_ts_utc` jest ustawiane tylko dla snapshotów `raw_json.kind=end_close`; zwykły latest/current snapshot dla aktywnej pozycji nie oznacza zamknięcia, więc UI nie pokazuje `end value` dla świeżego open.
- **Fix (2026-04-14, accounting):** Rebalance executor zapisuje deterministycznie `details.close_amount_a_raw` i `details.close_amount_b_raw` na evencie `bot_close_position` (best-effort świeży odczyt pozycji+pool tuż przed close; fallback do obliczonych amountów „before”). Lineage `node_metrics_from_lifecycle_best_effort` preferuje te pola przy liczeniu `end value`; `fee_payer_token_deltas` zostają tylko jako fallback dla starszych wierszy bez nowych pól.
- **Fix (2026-04-15, follow-up):** `persist_event_valuation_snapshots_for_positions` dla `end_close` nie wymaga już obecności `fee_payer_token_deltas`; najpierw bierze `details.close_amount_*_raw` i dopiero fallbackuje do delt. Dla `baseline_open` gdy brakuje jednej nogi puli w deltach, używa pełnego koszyka z `details.amount_*_cap` (obie nogi), żeby uniknąć mieszanego źródła (delta+cap) i zaniżenia start value. Dodatkowo `ON CONFLICT DO UPDATE` pozwala nadpisywać snapshoty `kind=end_close` (wcześniej warunek blokował aktualizację części zamknięć).
- **Fix (2026-04-15, recovery follow-up 2):** Recovery-like open (bot open + `rebalance_session_id` + wcześniejsze `bot_swap_*` w tej samej sesji) z niespójnymi pool-leg deltami (nie oba wydatkowe) wymusza pełny koszyk caps (`open_caps_recovery`) zamiast mieszania delta+cap.
- **Guards/tests:** `cargo test -p clmm-lp-api position_stream_lineage`; dodane: `pool_legs_strict_spend_requires_both_negative`, `open_row_is_recovery_like_when_prior_session_swap_exists`; po deploy: odśwież stream-lineage (persist) dla danej PDA — stary zły wiersz `baseline_open` może się zaktualizować tylko gdy trafi `open_caps`.
- **Residual risk:** Otwarcie **bez** `amount_*_cap` w `details` (np. część ścieżek CLI) — wtedy tylko delty + guardrail `node_metrics`; nadal możliwy drift cen/RPC. Dodatkowo możliwy pozostaje drift live/fallback przy chwilowej niedostępności RPC/price feed.
- **Symptom (2026-04-16, regression?):** Pozycja `52PR84ugSnNiaWbUAy1jrmf5YL7RqyzwvQmZ5u3wWEoC` pokazuje `start value ~$5.627` w historii (baseline z `amount_a_cap/amount_b_cap`), ale `Performance -> Value` pokazuje `~$0.218` (live valuation).
- **Paths:** `web/src/pages/PositionDetail.tsx` (Performance vs tabela lineage), `crates/api/src/handlers/positions.rs` (`get_position`, zapis snapshotów), `crates/api/src/services/position_stream_lineage.rs` (`node_metrics`, zapytania `position_stream_valuation_snapshots`), `crates/api/src/services/position_valuation.rs`

### BUG-20260413-05 — Stream lineage chained manual opens to unrelated rotation history

status: regressed  
severity: medium  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: position-stream-lineage, rotation, registry.jsonl, lifecycle, rebalance_session_id, false-parent, manual-open

- **Symptom:** A newly opened position in the same pool as prior activity appeared in **Position history (rotations)** as continuing an old PDA chain instead of a standalone node.
- **Symptom (2026-04-14):** Część nowych botowych rebalance (`close -> open`) nie dopina się do historii i pojawia się jako nowa pozycja startowa; jednocześnie manual opens na tym samym poolu muszą pozostać oddzielnymi historiami.
- **Symptom (2026-04-14, follow-up):** `linked positions` w strategiach rosły nieoczekiwanie; samo wejście na listę pozycji potrafiło modyfikować linki strategii (auto-heal w endpointach odczytowych).
- **Symptom (2026-04-14, follow-up 2):** Po ręcznym `Close` pozycja pozostawała w `position_addresses` strategii, mimo że manual close ma oznaczać koniec historii i brak dalszego zarządzania/reopen dla tej pozycji.
- **Symptom (2026-04-15, follow-up 3):** Recovery-open po `rebalance_incomplete` mógł utworzyć nowy PDA bez `rebalance_session_id`, więc lineage pokazywał nową pozycję jako osobny start-chain (bez powiązania z zamkniętym parentem), a strategia nie miała stabilnej kotwicy sesyjnej do old->new continuity.
- **Symptom (2026-04-15, follow-up 4):** Recovery-open tworzył nowy aktywny PDA, ale strategia pozostawała przypięta do starego `closed_position_nft` (`linked_strategies=[]` na nowej pozycji), co dawało niespójny status linku w UI.
- **Symptom (2026-04-15, follow-up 5):** Strategia z `parameters.position_addresses: []` (zamierzone „zarządzaj niczym”) nadal podejmowała decyzje na monitorowanych pozycjach, powodując częste rebalance nawet in-range.
- **Symptom (2026-04-16, follow-up 6):** Po `bot_open_position` dla `52PR84ug...` z `rebalance_session_id=facce...` stream-lineage potrafi zwrócić chain długości 1 zamiast kontynuacji po `4NL...` (open nastąpił wiele godzin po close; heurystyki parent inference mają okno ~60 min).
- **Root cause:** Registry fallback linked `registry_open` → `registry_close` by time/pool/owner even when `rebalance_session_id` did not match (or was empty). Lifecycle fallback linked opens to the latest close in a short window without requiring rotation evidence; forward close→open used the first qualifying open, not the true successor; lifecycle parent inference treated loose swap rows as rotation.
- **Root cause (runtime `debug-c45ac3.log`, H-lineage):** With DB enabled, `db_chain_from_edges_len` stayed **1** while JSONL fallback inflated `chain_len` to **3–4** — `build_linear_chain` walked from a single graph root and **omitted `entry` on forked `position_stream_edges`** (`A→B` and `A→C`), fell back to `[entry]`, then registry/lifecycle re-stitched a long pool history.
- **Root cause (2026-04-15, follow-up 3):** `recover_open_after_incomplete` called `open_new_range_with_wallet_mix(..., ledger_session_id=None)`; pending-open queue also did not persist session id. Recovered open rows were emitted as unanchored bot opens.
- **Root cause (2026-04-15, follow-up 4):** Pending-open success path added new position to monitor, but skipped normal post-rebalance continuity hooks (`managed_allowlist old->new` replacement and `reopen_hook`), so strategy `position_addresses` were not updated.
- **Root cause (2026-04-15, follow-up 5):** `StrategyExecutor::set_managed_allowlist` converted empty allowlist to `None`, which means unrestricted evaluation. This inverted semantics of explicit empty `position_addresses`.
- **Fix:** Registry parent/chain only when both rows carry the same non-empty `rebalance_session_id`. Lifecycle uses `lifecycle_rotation_parent_before_open` (session match, `close_kind=rotation`, or bot activity tied to the closed PDA); forward links require that helper to return the closed PDA; `infer_parent_position_from_lifecycle_best_effort` delegates to the same helper. DB mode uses **`build_lineage_chain_from_db_edges`** (backward from `entry`, then forward); **JSONL/registry fallback is skipped when any persisted edge touches `entry`**. **Update 2026-04-14:** JSONL suppress now blocks only fresh manual roots; bot-open nodes remain stitchable when rotation parent is inferable even with session mismatch. Rebalance executor now stamps one generated `ledger_session_id` through collect/close/swap/open lifecycle rows to improve deterministic close->open continuity.
- **Fix (follow-up):** Removed implicit strategy-link healing from read endpoints (`GET /positions`, diagnostics). Healing is now explicit via `POST /positions/{address}/heal-strategy-link`. Also hardened `replace_position_address_in_strategy` to only add/sync `new_position` when `old_position` was actually found/replaced, preventing accidental list growth on stale parent calls.
- **Fix (follow-up 2):** `DELETE /positions/{address}` now performs best-effort unlink from all strategies after successful manual close (`remove_position_address_from_all_strategies`), so operator close explicitly ends strategy linkage for that PDA.
- **Fix (follow-up 3):** Pending-open items now persist optional `rebalance_session_id`, `RebalanceResult` exposes generated session id for incomplete rebalance handoff, and recovery open passes that id into `open_new_range_with_wallet_mix` so recovered open rows carry the original session anchor.
- **Fix (follow-up 4):** On successful pending-open recovery, executor now performs the same continuity steps as standard rebalance success: replace `managed_allowlist` entry `old->new` (without growth) and call `reopen_hook(old, new)` to update strategy links.
- **Fix (follow-up 5):** `set_managed_allowlist` now keeps explicit empty list as restrictive (`Some(empty)`, target `0`) instead of unrestricted; added regression test `empty_managed_allowlist_stays_restrictive`.
- **Guards/tests (2026-04-14):** Added `jsonl_stitch_allowed_when_rotation_parent_exists_without_session_match`; existing `jsonl_stitch_suppressed_when_open_session_not_on_prior_close` and `jsonl_stitch_allowed_when_session_matches_prior_close` still pass.
- **Guards/tests:** Unit tests `lifecycle_chain_*`, `db_edges_*`, `jsonl_stitch_*`.
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`, `crates/execution/src/strategy/executor.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/pending_open.rs`

### BUG-20260413-04 — Open target USD looked too low after success

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: open-position, target-usd, valuation, wsol, usdc, price-source, drift

- **Symptom:** User sets open target (e.g. `5 USD`), position opens successfully, but displayed post-open value appears noticeably lower.
- **Root cause:** Runtime logs showed quote/open were near target, but later UI value used a lower external SOL/USD feed than pool-implied SOL/USD for the same WSOL/USDC pool state. This created an avoidable valuation drift in display value.
- **Fix:** For WSOL/USDC valuation path, backend now prefers SOL/USD implied from the pool tick (same on-chain state used for token amounts) instead of external feed-only pricing.
- **Guards/tests:** Verified on user reproduction after fix; instrumentation removed post-confirmation.
- **Paths:** `crates/api/src/services/position_valuation.rs`, `crates/api/src/handlers/pools.rs`, `crates/api/src/services/position_service.rs`, `crates/api/src/handlers/positions.rs`

### BUG-20260413-03 — Close Position fails with Whirlpool custom 6018

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: close-position, whirlpool, custom-6018, tokenminsubceeded, slippage, min-out

- **Symptom:** `Close Position failed: ... InstructionError(2, Custom(6018)) ... TokenMinSubceeded` on `whirLb...` even for manual close flow.
- **Root cause:** Manual close API path mapped executor failures to generic `500 Internal error`, so users did not get actionable guidance for Whirlpool `6018` (`TokenMinSubceeded`) even after executor-side retry logic was added.
- **Fix:** `PositionService::close_position` now classifies close errors (like open path): `6018`/slippage returns `400` with explicit min-out/slippage hint and suggested close-specific knobs (`--slippage-bps`, `WHIRLPOOL_CLOSE_SLIPPAGE_BPS`); wallet misconfiguration is mapped to `503` with signer setup hint.
- **Guards/tests:** Added regression test `close_position_error_6018_maps_to_bad_request_with_hint` in `crates/api/src/services/position_service.rs`; verified with `cargo test -p clmm-lp-api position_service::tests::close_position_error_6018_maps_to_bad_request_with_hint -- --nocapture`.
- **Paths:** `crates/protocols/src/orca/executor.rs`, `crates/api/src/services/position_service.rs`

### BUG-20260413-02 — PositionCreate nie sugerował swapu na operacyjny SOL

status: partially fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: position-create, swap-suggestion, operational-sol, rent, fees, jupiter

- **Symptom:** UI nie proponował `swap to SOL` przed `Open Position`, a backend zwracał błąd `API wallet has insufficient SOL for rent/fees`.
- **Symptom:** API potrafiło zwrócić `Top up both native SOL balance and wrapped-SOL token amount` mimo że UI nie blokował formularza (WSOL noga wyglądała na pokrytą).
- **Symptom:** `Open position failed: SPL Token program error InsufficientFunds` z logiem `Transfer: insufficient lamports ..., need ...` (Instruction 2) mimo wcześniejszego `Swap potwierdzony`.
- **Symptom:** Kolejne przypadki `InsufficientFunds` (Instruction 0) mimo że UI pokazywał wystarczające „Stan portfela”.
- **Symptom:** Nadal pojawiał się błąd `Transfer: insufficient lamports 24922302, need 33006413` (Instruction 0) nawet po wcześniejszych poprawkach UI.
- **Symptom:** Preflight bywa blokowany mimo wystarczającego `current_wsol` (tokenowo), bo native guard dla WSOL open działał w trybie konserwatywnym `need + pad`.
- **Root cause:** `fundingCheck` w `PositionCreate` sprawdzał tylko deficyty tokenów A/B dla notionalu pozycji i dla WSOL traktował dostępne saldo jako `native SOL + WSOL token`. Brakowało rozdzielenia wymagań: (1) WSOL token na nogę pozycji oraz (2) native SOL >= `min_open_lamports` na rent/fee buffer.
- **Root cause:** Dodatkowo brakowało doliczenia kosztu utworzenia konta WSOL (ATA rent) przy pierwszym użyciu WSOL, więc projekcja native SOL mogła być zaniżona względem realnego `SystemProgram::Transfer`.
- **Root cause:** UI walidował salda na podstawie lokalnie wybranego walleta (`/wallets/balances?owner=...`), podczas gdy `open/swap` są wykonywane przez API signer wallet z backendu. Przy rozjechanych portfelach frontend przepuszczał formularz mimo realnego braku środków na signerze.
- **Root cause:** W `WhirlpoolExecutor::preflight_open_liquidity_balances` pad operacyjny dla open był zbyt niski (`2_500_000` lamportów), więc preflight mógł przepuścić przypadki, które później padały na `SystemProgram::Transfer` po stronie tx build/send.
- **Fix:** Dodano check `shortOperationalSol` oparty o projekcję native SOL po finansowaniu nogi WSOL oraz `GET /wallets/api-signer (min_open_lamports)`. Dodatkowo `getAvailableUiAmount` liczy teraz saldo tokenowe (bez native), więc deficyt nogi WSOL jest wykrywany zgodnie z walidacją backendu. Projekcja odejmuje też estymatę rent dla WSOL ATA, gdy konto WSOL jeszcze nie istnieje. `blocked` uwzględnia oba warunki, a UI pokazuje dedykowany przycisk `Jupiter: swap to SOL`.
- **Fix:** `PositionCreate` przełączył źródło sald na API signer pubkey (z `/wallets/api-signer`) jako główny owner dla `wallet-balances`, walidacji fundingu i odświeżania po swapie. Dodano też notkę UI, że walidacja używa portfela API signer.
- **Fix:** Na podstawie historycznych openów z `data/ledger/orca_position_lifecycle.jsonl` (13 próbek: p50 ~10.08M, p95 ~10.48M, max ~11.07M lamportów) ustawiono domyślny `CLMM_MIN_OPEN_SOL_LAMPORTS` na `12_000_000` (0.012 SOL) jako bufor operacyjny. Wyższe wymagania SOL wynikające z notionalu nogi WSOL są walidowane oddzielnie.
- **Fix:** `WhirlpoolExecutor` używa teraz wspólnego bufora `open_native_sol_pad_lamports()` (env `CLMM_MIN_OPEN_SOL_LAMPORTS`, default `12_000_000`) zamiast sztywnego `2_500_000` w preflight WSOL/open; ten sam bufor jest też używany przy clampowaniu swapu WSOL.
- **Fix:** Usunięto blokujący heurystyczny bail native-SOL na etapie `preflight_open_liquidity_balances` dla nóg WSOL. Zamiast tego przed wysyłką tx wykonywana jest symulacja finalnego planu instrukcji (`simulate_transaction`) i parsowany jest rzeczywisty log runtime `Transfer: insufficient lamports X, need Y`; guard używa teraz `need` z symulacji + margines 1% (`ceil(need*1.01)`).
- **Symptom (2026-04-30, follow-up):** Na `PositionCreate` dla pary SOL/USDC UI nadal blokował `Open Position` komunikatem `Za mało tokenów...`, mimo że portfel miał wystarczający native SOL i brak WSOL był oczekiwany w modelu SOL-first.
- **Root cause (2026-04-30, follow-up):** Deficyt nóg A/B był liczony wyłącznie z bieżącego SPL token balance (`haveA/haveB`), więc noga WSOL wymagała pre-posiadania WSOL ATA zamiast uwzględnić wrap z native SOL przed open.
- **Fix (2026-04-30, follow-up):** `fundingCheck` w `PositionCreate` liczy teraz efektywne pokrycie nogi WSOL z native SOL (`native - min_open - ATA rent`) i dopiero to porównuje do `need*`; blokada token-deficit nie wymaga już dodatniego WSOL token balance, pozostaje osobny guard `shortOperationalSol`.
- **Guards/tests:** `npx tsc --noEmit` w `web/` przechodzi. TODO: test UI regresyjny dla scenariusza „A/B OK, ale operacyjny SOL za niski”.
- **Guards/tests:** `cargo check -p clmm-lp-protocols` przechodzi po zmianie prechecka na exact-plan + 1% margin.
- **Paths:** `web/src/pages/PositionCreate.tsx`, `crates/protocols/src/orca/executor.rs`

### BUG-20260413-01 — Manual close could trigger unintended reopen

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: manual-close, stale-snapshot, strategy-executor, evaluate_position, reopen, race

- **Symptom:** Po ręcznym `Close Position` chwilę później powstawała nowa pozycja, mimo że użytkownik nie uruchamiał rebalance.
- **Root cause:** `StrategyExecutor::evaluate_position` po `refresh_position` fallbackował do starego cache. Gdy monitor usunął już zamkniętą pozycję, ewaluator nadal mógł wykonać decyzję na starym snapshotcie.
- **Fix:** W `evaluate_position` usunięto fallback do stale cache dla ścieżki po refresh. Teraz:
  - przy błędzie refresh: skip cyklu (bez akcji),
  - gdy po refresh pozycji nie ma już w monitorze: skip cyklu (bez akcji).
- **Guards/tests:** TODO dodać test regresyjny race-case: manual close w trakcie pętli `evaluate_all` nie może wywołać `Rebalance`/`open`.
- **Paths:** `crates/execution/src/strategy/executor.rs`

### BUG-20260410-06 — Rotacja zamyka pozycję bez otwarcia nowej (urwany rebalance flow)

status: open  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in:   
keywords: rebalance, close_without_open, swap_mix, recovery, strategy

- **Symptom:** Pozycja zamknięta jako `close_kind=rotation`, ale brak kolejnego `bot_open_position` dla tej sesji/łańcucha.
- **Symptom (2026-04-16):** Dla sesji `facce436-7913-4173-b954-17f403d15a9d` (old PDA `4NLjjVqBtV4CVeFL224UzVpSW4Ds7g16rxuTvir78Qh3`) i `323c4f01-0526-484a-b9c4-ce820e4fc1e6` (old PDA `D6tnfq94B3WnAGeqVX3JUri9AhKfkfHNcNHviyTaBrcV`) recovery ma `attempts=13` i kończy się `open_position failed ... InstructionError(3, Custom(6012))` na `OpenPositionWithTokenExtensions`; wpisy pozostają w `data/pending-open-recovery.json` i w UI `Closed by bot, waiting for reopen`.
- **Symptom (2026-04-16, follow-up):** Bot otworzył nową pozycję `52PR84ugSnNiaWbUAy1jrmf5YL7RqyzwvQmZ5u3wWEoC` z `rebalance_session_id=facce436-7913-4173-b954-17f403d15a9d`, ale w UI nadal potrafi pozostać wpis w `Closed by bot, waiting for reopen` (prawdopodobnie przez pozostawiony item w `data/pending-open-recovery.json`, który generuje synthetic pending-only row mimo że lifecycle ma `bot_open_position`).
- **Symptom (2026-04-17, follow-up):** Dla sesji `2c0ab4d9-dcc6-485d-8beb-ce4a5910365a` (old PDA `DfjqibKyfMtXqkZrfsfmWvbxZxdZTH6m6J1L5qKnv4Xq`) pending-open recovery rośnie do `attempts=68` z błędem `pool tick -24299 not in new range [-24264, -24160): cannot quote deposit for open`; `open_seen=false`, pozycja pozostaje w `Closed by bot, waiting for reopen`.
- **Symptom (2026-04-17, quality follow-up):** Przy długim recovery operator nie miał standaryzowanego pola `stuck_reason` i progu alertu `attempts > N`; diagnoza opierała się na ręcznym czytaniu `last_error`.
- **Root cause:** Rebalance flow urwał się po close + etapach swap (`bot_swap_mix_round` / `bot_swap_exact_in_attempt`) bez finalnego open; brak jawnego `rebalance_incomplete` wpisu dla tego przypadku.
- **Root cause (follow-up 2026-04-17):** Recovery używa sztywno `intended_tick_lower/upper` z momentu close; gdy rynek przesunie się poza ten zakres, `quote_deposit_budget_in_range` odrzuca open (`tick_current` poza nowym pasmem). Ścieżka `recover_open_after_incomplete` nie stosuje adaptacji zakresu (widen/recenter), więc ponawia ten sam niepoprawny zakres.
- **Fix:** Do wdrożenia: twardy marker `rebalance_incomplete` + trwały `pending-open` recovery gdy close zakończony, a open nie doszedł do skutku; UI powinno pokazywać taki status zamiast "po prostu closed".
- **Fix (2026-04-17, quality):** `pending-open` zapisuje telemetry per item (`last_attempt_at`, `stuck_reason`, `stuck_since`, `last_alert_attempts`) i klasyfikuje `stuck_reason` automatycznie z `last_error` (`tick_out_of_range`, `quote_failed`, `rpc_timeout`, `insufficient_balance`, `unknown`). Dodano próg `CLMM_PENDING_OPEN_ALERT_ATTEMPTS` (default 10): po przekroczeniu emitowany jest alert `Pending Open Stuck` (z deduplikacją per item).
- **Symptom (2026-04-27, follow-up):** Dla `retouch_shift` finalny range open po recovery mógł odbiegać od pierwotnego planu strategii po kilku minutach opóźnienia (stary plan z momentu close był wykonywany na nowym rynku).
- **Symptom (2026-04-30, follow-up):** `Closed by bot, waiting for reopen` potrafiło dalej pokazywać sesję mimo że lifecycle miał już `bot_open_position` dla tego samego `rebalance_session_id`; równolegle pending-open wielokrotnie wpadał w guard `session_already_has_open_row`.
- **Root cause (2026-04-27, follow-up):** `pending-open` nie trzymał metadanych świeżości planu (`planned_at`, `planned_price`) i recovery otwierał na starych `intended_tick_*` bez jawnej polityki stale/drift replan.
- **Root cause (2026-04-30, follow-up):** Queue `pending-open-recovery` nie była samoczyszcząca dla sesji już otwartych; synthetic `pending-only` rows w watchdogu opierały się na samym stanie kolejki, więc mogły utrwalać „stare duchy” po udanym reopenie.
- **Fix (2026-04-27, follow-up):** `pending-open` zapisuje teraz `planned_at_utc` i `planned_price_ab`. Recovery dla `RetouchShift` sprawdza TTL (`CLMM_RECOVER_PLAN_TTL_SECS`, default 180s) i drift ceny (`CLMM_RECOVER_PLAN_MAX_DRIFT_PCT`, default 1%). Przy stale/drift replanuje zakres (zachowując szerokość) wokół bieżącego ticka, loguje `bot_recover_open_replanned`, i zapisuje `range_adjustment_reason` do zdarzenia rebalance.
- **Fix (2026-04-30, follow-up):** Executor przed próbą recovery sprawdza `session_has_bot_open_position(rebalance_session_id)` i automatycznie usuwa z kolejki przeterminowany pending item (bez kolejnych prób). Watchdog snapshot pomija synthetic `pending-only` dla sesji, które mają już `bot_open_position`.
- **Fix (2026-04-30, follow-up 2):** Klasyfikacja `stuck_reason` rozpoznaje teraz Whirlpool `Custom(6012)`/`0x177c` jako `open_position_6012` zamiast `unknown`, co daje jednoznaczny sygnał operacyjny.
- **Symptom (2026-04-30, follow-up 3):** Po udanym recovery-open nowa pozycja potrafiła mieć wartość ~`<1 USD` mimo wcześniejszego targetu ~`10 USD`.
- **Symptom (2026-04-30, follow-up 4):** Po zmianach SOL-first część reopenujących flow traciła skuteczność; po swap-mix bot potrafił nie domknąć sensownego depozytu dla open/reopen.
- **Root cause (2026-04-30, follow-up 3):** Gdy recovery nie odczytał `close_amount_{a,b}_raw` z lifecycle, stosował fallback `(1,1)`, co dawało skrajnie niski `prev_end_value_usd` i zaniżony `target_usd`.
- **Root cause (2026-04-30, follow-up 4):** SOL-first auto-unwrap po `swap_exact_in` działał także gdy swap **kupował WSOL** (np. leg B->A w swap-mix), więc świeżo kupiony WSOL mógł być od razu odwijany do native SOL przed kolejnym krokiem open.
- **Fix (2026-04-30, follow-up 3):** Recovery fallback dla brakujących close amounts zmieniono na `(0,0)` + jawny warning; przy `prev_end<=0` sizing przechodzi na `wallet_cap` (`target_usd_from_prev_end_clamped`) zamiast mikro-notionalu.
- **Fix (2026-04-30, follow-up 4):** `swap_exact_in` wykonuje auto-unwrap tylko gdy `specified_mint == WSOL` (sprzedaż WSOL). Dla swapów kupujących WSOL cleanup jest pomijany, żeby nie niszczyć miksu tokenów wymaganego przez natychmiastowy open/reopen.
- **Guards/tests:** test scenariusza: close success + swap rounds + open failure/abort => wpis `rebalance_incomplete` + recovery artifact; dodatkowo testy klasyfikacji `stuck_reason` i progowego alertowania attempts.
- **Guards/tests (2026-04-30, follow-up 3):** `cargo test -p clmm-lp-execution target_usd_uses_wallet_cap_when_prev_end_unknown_or_zero -- --nocapture`, `cargo check -p clmm-lp-execution`.
- **Guards/tests (2026-04-30, follow-up 4):** `cargo check -p clmm-lp-protocols`.
- **Paths:** `data/ledger/orca_position_lifecycle.jsonl`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/pending_open.rs`, `crates/execution/src/strategy/executor.rs`, `crates/execution/src/lifecycle/events.rs`, `crates/api/src/handlers/positions.rs`

### BUG-20260410-05 — Collect Fees: brak executora mimo aktywnego środowiska

status: partially fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in: local (pending user verification)  
keywords: collect_fees, executor, wallet, KEYPAIR_PATH, SOLANA_KEYPAIR, WALLET_KEYPAIR_BASE58, fee_owed, position-reader

- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee; (4) w lineage widoczne `collect 1x` oraz `A=0/B=0` bez jasnej informacji czy to błąd czy faktycznie zero owed.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee; (4) w lineage widoczne `collect 1x` oraz `A=0/B=0` bez jasnej informacji czy to błąd czy faktycznie zero owed; (5) `LP Zebrane`: jedna noga miała wartość, druga pokazywała `-`.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee; (4) w lineage widoczne `collect 1x` oraz `A=0/B=0` bez jasnej informacji czy to błąd czy faktycznie zero owed; (5) `LP Zebrane`: jedna noga miała wartość, druga pokazywała `-`; (6) brak szybkiego wyjaśnienia „dlaczego 0” w UI.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee; (4) w lineage widoczne `collect 1x` oraz `A=0/B=0` bez jasnej informacji czy to błąd czy faktycznie zero owed; (5) `LP Zebrane`: jedna noga miała wartość, druga pokazywała `-`; (6) brak szybkiego wyjaśnienia „dlaczego 0” w UI; (7) komunikat collect bez czytelnego formatu oraz brak widoku collect leg values w tabeli sesji.
- **Symptom:** (8) `Internal error: stream lineage: collect fee rows: error returned from database: kolumna "lp_collected_token_a_raw" nie istnieje` na środowiskach z nieodpaloną migracją `005_ledger_lp_collected_raw.sql`.
- **Symptom (2026-04-17, follow-up):** `positions/{pda}/lifecycle-summary` i karty kosztów/prowizji potrafią pokazywać same zera (`tx=0`, `collect=0`) mimo że `bot-activity/ledger` dla tego samego PDA zawiera `bot_open_position`/`bot_collect_fees`/`bot_close_position` z dodatnimi `tx_fee_lamports` i legami collect.
- **Root cause:** Dwa etapy: (a) resolver executora ładował wallet tylko z części źródeł env i proces API nie dziedziczył signer vars; (b) collect path twardo failował gdy pre-read pozycji do `fee_owed_*` nie powiedzie się, mimo że sama transakcja collect może być wykonalna.
- **Root cause:** (c) zapytanie lineage zakładało fizyczne kolumny `lp_collected_token_*_raw` w `position_stream_ledger_rows`; na starszym schemacie były tylko w `raw_json`.
- **Root cause (follow-up 2026-04-17):** Dwa niezależne problemy regresyjne: (d) w `lifecycle-summary` matching był de facto `session OR ELSE position` (if/else), więc wiersze z obcym `rebalance_session_id` były odrzucane nawet gdy `position_pubkey` pasował do streamu; (e) ingest lifecycle->DB zakładał nowe kolumny (`fee_payer_token_deltas`, `lp_collected_token_*_raw`) i na starszym schemacie kończył się błędem, przez co agregacje DB pozostawały puste.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data`.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data`. Dla lineage dodano notę jakości danych: przy `collect_events > 0` i `A/B == 0` API jawnie komunikuje, że collect został wykonany przy `fee_owed_a/b == 0`.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data`. Dla lineage dodano notę jakości danych: przy `collect_events > 0` i `A/B == 0` API jawnie komunikuje, że collect został wykonany przy `fee_owed_a/b == 0`. Jeśli collect ma tylko jedną nogę zmapowaną, brakująca noga jest normalizowana do `0` (z notą), aby UI nie pokazywał `-`.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data`. Dla lineage dodano notę jakości danych: przy `collect_events > 0` i `A/B == 0` API jawnie komunikuje, że collect został wykonany przy `fee_owed_a/b == 0`. Jeśli collect ma tylko jedną nogę zmapowaną, brakująca noga jest normalizowana do `0` (z notą), aby UI nie pokazywał `-`. Dodano per-node `collect_zero_diagnostics` (in-range share est., swap count est., position share est.) i render w tabeli `LP Zebrane`. Dodatkowo LP legs dla collect są teraz brane priorytetowo z Orca `harvest_position_instructions.fees_quote` (obie nogi), a nie tylko z pre-read `PositionReader`.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data` (kwoty w komunikacie zaokrąglone do 3 miejsc). Dla lineage dodano notę jakości danych: przy `collect_events > 0` i `A/B == 0` API jawnie komunikuje, że collect został wykonany przy `fee_owed_a/b == 0`. Jeśli collect ma tylko jedną nogę zmapowaną, brakująca noga jest normalizowana do `0` (z notą), aby UI nie pokazywał `-`. Dodano per-node `collect_zero_diagnostics` (in-range share est., swap count est., position share est.) i render w tabeli `LP Zebrane`. Dodatkowo LP legs dla collect są teraz brane priorytetowo z Orca `harvest_position_instructions.fees_quote` (obie nogi), a nie tylko z pre-read `PositionReader`. W tabeli sesji (`Logs / rebalances`) dodano kolumnę `Collect values` z `A raw/B raw` dla collect tx.
- **Fix:** (2026-04-13) Query w `stream-lineage` został uodporniony na drift schematu: wartości `lp_collected_token_*_raw` są czytane z `raw_json` (aliasowane do tych samych nazw), bez bezpośredniego odwołania do brakujących kolumn.
- **Fix (2026-04-17):** `lifecycle-summary` używa teraz rzeczywistego OR (`session match` **lub** `position match`) i ma regresyjny test. Ingest `position_stream_ledger_rows` dostał detekcję kolumn `information_schema` i zapisuje fallback-variant SQL zgodny ze starszym schematem (bez optional columns), zamiast cicho tracić cały ingest.
- **Guards/tests:** dodać unit testy resolvera źródeł wallet (ścieżka vs env key material) i test collect_fees dla scenariusza „position pre-read fails but tx still executes”.
- **Paths:** `crates/api/src/services/position_executor.rs`, `crates/api/src/services/position_service.rs`, `crates/api/src/handlers/wallets.rs`, `tools/Start-ClmmApi-8081.ps1`, `crates/execution/src/strategy/rebalance.rs`, `crates/api/src/services/position_stream_lineage.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/services/position_stream_performance.rs`

### BUG-20260410-04 — Brak regresyjnych testów UI dla feedbacku collect/swap

status: open  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in:   
keywords: ui-tests, collect_fees, swap-before-open, message-passthrough, regression

- **Symptom:** Zmiany w komunikatach UI dla `Collect Fees` i `Swap` mogą wracać do starego, mylącego zachowania bez szybkiego wykrycia.
- **Root cause:** Brak dedykowanych testów integracyjnych frontend dla scenariuszy „success bez signature” / „backend message passthrough”.
- **Fix:** Dodać testy UI/integration dla `PositionDetail` i `PositionCreate` weryfikujące prezentację `message` z API.
- **Guards/tests:** PR touching `web/src/pages/PositionDetail.tsx` lub `web/src/pages/PositionCreate.tsx` powinien zawierać test dla tych ścieżek.
- **Paths:** `web/src/pages/PositionDetail.tsx`, `web/src/pages/PositionCreate.tsx`

### BUG-20260410-01 — Lineage showed inconsistent start/end for same node

status: open  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in: local  
keywords: lineage, baseline_value_usd, current_value_usd, rebalance_session_id, rotation

- **Symptom:** same PDA showed different `start/end` across views; sometimes `start` wildly above `end`.
- **Symptom:** `Logs / rebalances` can show `—` in `start value` / `end value` for older rows (or intermittently for the same row) because API emits `0` for baseline/current in some runs.
- **Root cause:** mixed-source lineage valuation with fallback drift.
- **Root cause:** per-node valuation in `stream-lineage` still depends on short-timeout best-effort price fetch (`fetch_mint_prices_usd` with 2s timeout) and incomplete open/close leg deltas in lifecycle rows; when either is missing, baseline/current may remain zero and UI intentionally renders `—` for zero.
- **Fix:** session-first continuity (`close(old)->open(new)` by same `rebalance_session_id`) plus guardrails on cap/continuity fallbacks.
- **Fix:** Added stable mint-price merge in lineage (`live + last-good cache`, TTL 15m) so temporary free-price timeouts do not zero `start/end` for rows that had recent quotes; backfill snapshots now use the same stable price path.
- **Fix:** `stream-lineage` now persists and prefers event snapshots (`baseline_open` / `end_close`) for chain PDAs in DB mode, with token amounts + USD + `price_source` + `valuation_quality` in snapshot `raw_json`; baseline/current selection prioritizes these kinds over generic earliest/latest snapshots.
- **Guards/tests:** Added unit invariants in `position_stream_lineage.rs` (`continuity_from_session_carries_prev_end_to_next_baseline`, `baseline_fallback_guardrail_blocks_implausible_prev_end`).
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`

### BUG-20260410-02 — Collect fees UI said requested, but no real effect

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in: local  
keywords: collect_fees, ui-message, dry_run, position_detail

- **Symptom:** UI displayed `Collect requested.` even when backend returned dry-run/no-op style response.
- **Root cause:** frontend success toast ignored backend message payload.
- **Fix:** `PositionDetail` now shows API `message` for collect, not hardcoded text.
- **Guards/tests:** still missing dedicated UI integration test for collect message passthrough (open action item).
- **Paths:** `web/src/pages/PositionDetail.tsx`

### BUG-20260410-03 — Swap step appeared to do nothing

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in: local  
keywords: swap-before-open, position-create, dry-run, ui-feedback

- **Symptom:** clicking `Swap` often looked like no action.
- **Root cause:** backend dry-run/info response was not surfaced when signature was missing.
- **Fix:** `PositionCreate` now renders `swapStepInfo` from API message regardless of signature presence.
- **Guards/tests:** still missing dedicated UI integration test for swap message without `swap_signature` (open action item).
- **Paths:** `web/src/pages/PositionCreate.tsx`

