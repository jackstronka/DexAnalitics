# Plan: materializacja „Historii pozycji” w Postgres (`position_chain_history_nodes`)

**Cel produktowy:** dane pokazywane w UI (siatka lineage / rotacje, sumy łańcucha) mają być **wstępnie policzone i zapisane** w Postgresie, a API ma je **szybko odczytywać** — bez wielu sekund obliczeń na żądanie przy długich łańcuchach.

**Strategia wdrożenia (zatwierdzona kierunkowo):** **ścieżka równoległa** do obecnego `GET /api/v1/positions/{address}/stream-lineage` (compute-on-read). Nowa ścieżka powstaje i jest testowana osobno; **przełączenie UI** dopiero gdy read-model jest wiarygodny i szybki.

**Stan implementacji (2026-05-14):** migracje **007 + 008** (`metrics_mode` na wierszach + tabela `position_chain_history_meta` na `chain` / `totals` / `chain_cost_summary`). **Writer + API:** `POST /api/v1/positions/{address}/chain-history/refresh` (materializuje przez `compute_position_stream_lineage`, opcjonalnie `mode=settlement_v1` jak w stream-lineage), `GET /api/v1/positions/{address}/chain-history` (odczyt z PG; **404** gdy brak zapisu). Kod: `crates/api/src/services/position_chain_history.rs`, handlery w `handlers/positions.rs`. **UI:** `getPositionLineagePreferMaterialized` — najpierw chain-history, fallback stream-lineage; **P4:** badge + krótki opis ścieżki API na kartach „Historia pozycji” (`PositionDetail`, `ClosedPositionDetail`). Klient TS (`web/src/lib/api.ts`): `getPositionChainHistory`, `refreshPositionChainHistory`, oraz **`txBuildUnsigned` / `txSubmitSigned` / `chainHistoryAnchorsFromTxBuild`** dla przepływu unsigned build → podpis → submit z kotwicami chain-history.

**Stan historyczny (przed GO):** migracja 007 + runner; brak writera i endpointów — zastąpione powyżej.

Powiązane: [`DATA_CATALOG.md`](DATA_CATALOG.md), [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md), [`BUGS.md`](BUGS.md) `BUG-20260512-02`.

---

## 1. Problem i metryki sukcesu

| Problem | Docelowo |
|--------|----------|
| `stream-lineage` przy dużej liczbie PDA / zapytań do DB + cen wykonuje ciężką pracę w handlerze HTTP | Odczyt „Historii” z PG w **< 500 ms** typowo (lokalnie), przy założeniu indeksu po `chain_anchor_pubkey` i braku joinów RPC w hot path |
| Ten sam wynik liczony wielokrotnie przy każdym odświeżeniu UI | **Idempotentny zapis** (UPSERT po `(chain_anchor_pubkey, chain_seq)` / `(chain_anchor_pubkey, position_pubkey)`) |
| Ryzyko regresji przy przełączeniu | UI i operator mają **fallback** na obecny compute do czasu pełnej zgody |

**Kryteria ukończenia fazy „produkcyjnej”:** (a) writer działa w kontrolowanych triggerach, (b) endpoint read zwraca strukturę zgodną z kontraktem (§4), (c) testy integracyjne lub testy writera + smoke API, (d) wpis w `BUGS.md` / `ENGINEERING_NOTES` przy zmianie domyślnego źródła dla UI.

---

## 2. Zakres danych (MVP vs później)

### MVP (wystarczy na pierwszą wersję UI „z cache”)

- Wiersze odpowiadające **`PositionStreamLineageNode`** w sensie biznesowym: PDA, czasy open/close (best-effort), baseline/current (jako `start_value_usd` / `end_value_usd` / `current_value_usd` wg semantyki wiersza w 007), `tx_fee_lamports`, `tx_fees_usd`, `collect_events`, `fees_collected_*`, `realized_cashflow_usd`, `net_pnl_*`, minty, opcjonalnie `range_label_at_open` / `close_price_label`.
- Kolejność łańcucha: `chain_seq` rosnąco = **od najstarszej do najnowszej** (zgodnie z komentarzem w migracji 007).
- **`chain_anchor_pubkey`:** PDA użyte jako wejście (np. URL) w momencie materializacji — pozwala na **re-materializację** bez zmiany klucza UI.

### Poza MVP (iteracja 2+)

- Pełna kopia odpowiedzi API w `raw_snapshot` JSONB (audyt / replay) — kolumna już istnieje.
- Zmaterializowane **totals** / `chain_cost_summary` — zapis w **`position_chain_history_meta`** (`totals_json`, `chain_cost_summary_json`); węzły w `position_chain_history_nodes`.
- Ewentualna migracja **008+** jeśli brakuje pól (np. dedykowana kolumna pod metrykę „principal Δ” jeśli nie mieści się w `principal_delta_usd` + `raw_snapshot`) — zgodnie z checklistą w `ENGINEERING_NOTES`.

---

## 3. Architektura (komponenty)

```
[ Źródła prawdy już dziś ]     lifecycle JSONL, IL ledger, DB snapshots, RPC (ograniczone)
            │
            ▼
   ┌────────────────────┐
   │ Writer (job/API)  │  wywołuje istniejącą logikę `compute_position_stream_lineage`
   │                    │  (lub wewnętrzny builder dzielący moduły) → mapuje wynik → UPSERT
   └─────────┬──────────┘
             ▼
   ┌─────────────────────────────┐
   │ position_chain_history_nodes │
   └─────────┬───────────────────┘
             ▼
   ┌────────────────────┐
   │ GET … (nowy lub    │  preferuje PG; opcjonalnie `?source=compute` dla debug
   │  query na istniejącym) │
   └────────────────────┘
             ▼
          [ UI ]
```

**Zasada:** writer **nie duplikuje** heurystyk łańcucha od zera w pierwszej iteracji — **re-używa** `compute_position_stream_lineage` (lub wyciągnięte funkcje pomocnicze), żeby wynik materializowany = wynik referencyjny przy tych samych wejściach.

---

## 4. Kontrakt API (**wdrożone** — Opcja A)

- **`POST /api/v1/positions/{address}/chain-history/refresh`** — przelicza lineage (`compute_position_stream_lineage`), opcjonalnie `mode=settlement_v1` dla totals jak w `stream-lineage`; zapisuje wiersze + meta w transakcji (DELETE + INSERT).
- **`GET /api/v1/positions/{address}/chain-history`** — odczyt z PG (`?mode=settlement_v1` | domyślnie `live`). **404** gdy brak materializacji dla pary (anchor, mode). Payload: **`PositionStreamLineageResponse`** (węzły z deserializacji `raw_snapshot` per wiersz).

**OpenAPI / Swagger:** zarejestrowane w `openapi.rs`.

**Uwaga bezpieczeństwa:** gdy ustawisz **`CLMM_CHAIN_HISTORY_REFRESH_SECRET`** w env API, `POST …/chain-history/refresh` wymaga nagłówka **`Authorization: Bearer <sekret>`** albo **`X-Chain-History-Refresh: <sekret>`** (patrz `.env.example`). Bez zmiennej — zachowanie dev (bez dodatkowego nagłówka). Dalsze zaostrzenie: sieć prywatna / reverse proxy.

---

## 5. Writer: kiedy uruchamiać

| Trigger | Opis |
|--------|------|
| **Po udanym** `compute_position_stream_lineage` w ścieżce admin/CLI | Jawny `POST …/chain-history/refresh` (API) lub **`clmm-lp-cli chain-history-refresh <anchor>`** (HTTP do działającego API; patrz `--help`) |
| **Po mutacji pozycji przez API** | Po sukcesie (nie dry-run): `open_position` (gdy jest PDA), `close_position`, `collect_fees`, `decrease_liquidity`, `rebalance_position` — w tle `materialize_chain_history_for_anchor` (`live`; opcjonalnie drugi pass `settlement_v1` gdy `CLMM_CHAIN_HISTORY_TRIGGERS_SETTLEMENT_V1=1`). Wyłączenie: `CLMM_CHAIN_HISTORY_TRIGGERS=0` lub legacy `CLMM_CHAIN_HISTORY_CLOSE_TRIGGER=0`. |
| **Unsigned tx flow** (`POST /api/v1/tx/*/build` → podpis portfela → **`POST /api/v1/tx/submit-signed`**) | API **nie** parsuje podpisanej transakcji w poszukiwaniu PDA. Klient może w body submitu podać **`chain_history_anchors`**: lista pubkey (np. **`position_address`** zwrócony z buildu dla open/increase). Po **udanym** `send_transaction` każda niepusta kotwica uruchamia ten sam background co powyżej (trigger **`tx_submit_signed`**). W web dashboardzie: `txSubmitSigned(..., { build })` w `web/src/lib/api.ts` scala `position_address` z odpowiedzi builda z jawnymi anchorami. |
| **Strategia / bot (`RebalanceExecutor`)** | Po sukcesie on-chain w `ensure_execution_success`: open → `created_position`; close / collect / decrease / swap (gdy podano PDA do ledgera) → `position`. Trigger logów: **`strategy_executor`**. Konfiguracja wyłączenia jak w wierszu „Po mutacji pozycji przez API”. |
| **Cron / backfill** | Jednorazowe wypełnienie dla listy PDA z produkcji (skrypt + limit rps) |

**Reguła nadpisywania (MVP):** `ON CONFLICT DO UPDATE` z aktualizacją wszystkich kolumn liczonych + `materialized_ts_utc = now()`. Dyskusja: czy **kasować** wiersze dla anchorów, których łańcuch się skrócił (rzadkie) — na MVP: **pełny rewrite** łańcucha w transakcji (DELETE WHERE anchor = X; INSERT batch) prostszy niż diff.

---

## 6. Spójność, jakość, bezpieczeństwo

- **Źródło semantyki:** dokumentować w odpowiedzi API, że read-model jest **best-effort** tak jak `stream-lineage` (te same definicje baseline/current).
- **Tryb metrics:** jeśli UI używa `mode=live` vs settlement — writer musi zapisywać **etykietę trybu** (np. nowa kolumna `metrics_mode TEXT` w 008 lub pole w `raw_snapshot`) albo **osobny zestaw wierszy** per tryb — **decyzja przed pierwszym zapisem produkcyjnym**.
- **Uprawnienia:** endpoint `refresh` tylko dla roli operator / env flag w dev.

---

## 7. Fazy realizacji (kolejność PR)

| Faza | Dostarczane | Test / definicja „done” |
|------|-------------|-------------------------|
| **P0** | Moduł writera `materialize_chain_history_for_anchor` + mapowanie wierszy + `raw_snapshot` | ✅ `position_chain_history.rs` + unit testy pomocnicze |
| **P1** | `POST …/chain-history/refresh` + `tracing` | ✅ handler + route |
| **P2** | `GET …/chain-history` | ✅ handler + route; brak dedykowanego testu integracyjnego z prawdziwym PG w CI |
| **P3** | Web: prefer `chain-history`, fallback `stream-lineage` | ✅ `getPositionLineagePreferMaterialized` + `queryKey` `position-lineage` |
| **P4** | Domyślne źródło w UI / auth na refresh | **Częściowo:** badge źródła odczytu (PG vs compute) + i18n opis ścieżki; refresh chroniony opcjonalnym `CLMM_CHAIN_HISTORY_REFRESH_SECRET` (patrz §4). **Do decyzji:** wymuszenie wyłącznie compute z UI (`?source=compute`) — nie wdrożone. |

---

## 8. Świadome „nie w zakresie” pierwszej iteracji

- Zastąpienie **wszystkich** ciężkich zapytań dashboardu (portfolio, effective-balances) — ten plan dotyczy **lineage / Historia pozycji**.
- Gwarancja identyczności co do centa z przyszłym backtestem — nadal best-effort zgodnie z regułami repo o fee/proxy.

---

## 9. Checklist przed merge „włączamy UI na PG”

- [ ] Porównanie próbek: ≥ N anchorów, łańcuchy krótkie i długie (`chain.len()` > 8).
- [ ] Zapisany `metrics_mode` zgodny z UI.
- [ ] Dokumentacja OpenAPI + jedna linia w `FUNCTIONAL_SPECIFICATION.md` (opcjonalnie § od read-model lineage).
- [ ] `keywords:` w `ENGINEERING_NOTES` + ewentualna aktualizacja statusu w `BUGS.md` jeśli zamykamy wątek latency przez materializację.
