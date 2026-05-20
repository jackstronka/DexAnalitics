# Wallet GL — wizja księgowa vs stan implementacji (plan)

**Cel dokumentu:** jedno miejsce w repo dla wymagań „GL jak w księgowości”, żeby **nie powtarzać wątku czatu** między sesjami AI / ludźmi. Odwołanie w promptach: *„zgodnie z `doc/WALLET_GL.md`”*.

**Powiązane:** [`DATA_CATALOG.md`](DATA_CATALOG.md) (`data/wallet-ledger-events.jsonl`), [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md) (wpisy `wallet_ledger`), UI `/wallet/ledger`, endpoint `GET /api/v1/wallets/ledger-events`. **Norma produktowa (salda UI vs GL):** [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md) **§5** i **§5.1**.

---

## 1. Wizja produktowa (normatywna — docelowo)

**Cytat intencji z wątku (skrócona treść merytoryczna):**

- GL ma działać **jak w systemie księgowym**: **każda transakcja** dopisuje lub odpisuje wartości z GL (**delty na konta**).
- **Nie liczymy za każdym razem od zera** z łańcucha — utrzymujemy **bieżący stan** przez **sumowanie (lub inkrementalny agregat) ruchów na kontach**, o ile księgowanie jest **kompletne i poprawne**.
- Wtedy **stan portfela jest „na wyciągnięcie ręki”** z GL, **bez ciągłego zaciągania on-chain** — z zastrzeżeniem **reconcile** (porównanie z on-chain / snapshotem) jako kontrola jakości, niekoniecznie przy każdym odświeżeniu UI.

**Implikacje techniczne (muszą być jawne w kodzie i w UI):**

| Wymóg | Uzasadnienie |
| ----- | ------------ |
| **Kompletność zdarzeń** | Brak jednej operacji = rozjechany stan względem rzeczywistości. |
| **Plan kont (chart of accounts)** | Jednoznaczne mapowanie: mint / natywny SOL / opłaty tx / „w puli” vs „w portfelu” itd. |
| **Konwencja znaku i jednostki** | `raw_delta_i128` per mint + ewentualnie osobne konto na fee; brak chaosu co jest debetem. |
| **Punkt startowy (opening balance)** | Sam GL bez stanu początkowego nie zastępuje łańcucha; start: snapshot on-chain lub zamknięcie poprzedniego okresu. |
| **Obsługa błędów i pending** | `pending` → `confirmed` / `failed`; brak „ciszy” po nieudanym submit. |
| **Reconcile** | Okresowo lub na żądanie: GL vs RPC — jawna metryka rozbieżności (jak w księgowości). |

---

## 2. Stan obecny w kodzie (faza A — *journal*, nie pełny ledger)

**Plik:** `data/wallet-ledger-events.jsonl` (override: `CLMM_WALLET_LEDGER_PATH`).

**Co jest zrobione:**

- Append-only **dziennik zdarzeń** API: `schema_version`, `ts_utc`, `event_id`, `correlation_id`, `status` (`pending` / `confirmed` / `failed`), `kind`, `owner`, `signature`, opcjonalnie `pool_address`, `cost_session_id`, `native_lamports_delta`, `deltas[]`, `error`, `source`.
- **Spięte ścieżki (nie „każda transakcja”):** m.in. `swap_before_open`, `open_position`, `close_position`, `collect_fees` (delty z pre/post snapshotów UI → raw mint), `decrease_liquidity`, `rebalance_position` (journal zdarzenia; delty tokenów dopiero gdy będzie dekodowanie/sim), `transfer_sol`, `convert_sol` (w tym pending → outcome dla transfer/convert tam, gdzie wdrożone).
- **Odczyt:** `GET /api/v1/wallets/ledger-events` (query: `owner`, `kind`, `status`, `limit`; pole `storage`: `postgres` | `jsonl` | `jsonl_fallback`); **UI:** `/wallet/ledger`.
- **Dual-write (2026-05-15):** każdy append idzie do JSONL **i** (gdy Postgres połączony) do tabeli **`wallet_gl_journal_event`** (migracja `010_*`). Odczyt preferuje Postgres gdy są wiersze, inaczej JSONL.

**Czego świadomie *nie* robi obecny kod:**

- Nie utrzymuje **salda per konto** wyłącznie z sumy wierszy GL.
- **Effective wallet / salda UI** nadal opierają się na **RPC + cache** (`effective-balances` itd.), **nie** na replice z GL.
- Nie obejmuje jeszcze **wszystkich** typów operacji API (np. ścieżki `tx/*` submit, `increase_liquidity` — do dopięcia w fazie B+ / C).

### 2.1 Decyzja architektoniczna: PostgreSQL jako docelowy magazyn GL

**Ustalenie (2026-05-14):** warstwa księgowa **powyżej pliku JSONL** — w szczególności **plan kont**, **trwały journal / wpisy ledgera** (append-only w sensie biznesowym) oraz **read model sald** (Fazy C–D) — ma być realizowana w **PostgreSQL** (ten sam ekosystem co `position_stream_*`, migracje w `crates/data/migrations/`).

- **JSONL (`wallet-ledger-events.jsonl`)** pozostaje **bieżącym** źródłem zapisu do czasu migracji; docelowo: albo **dual-write** (JSONL + Postgres), albo **wyłącznie Postgres** po backfillu i przełączeniu odczytów (`GET /wallets/ledger-events`).
- **Pierwszy krok Postgres (2026-05-14):** tabele seed **`wallet_gl_token_account`** (unikalny mint = konto) oraz **`wallet_gl_curated_pool`** (pary z `curated_backtest_pools`) — migracja `009_wallet_gl_curated_tokens_and_pools.sql`.
- **Uzasadnienie:** ACID, zapytania po `owner` / `correlation_id` / czasie, indeksy, jedna baza backupów/operacji, spójność z typowymi systemami GL w fintech (zob. dyskusja w repo / `ENGINEERING_NOTES`).

Szczegóły migracji (nazwy tabel, kolejność rollout) dopisujemy przy pierwszym PR ze schematem — bez „cichego” rozszerzania zakresu poza tę decyzję.

### 2.2 Konto logiczne per cykl życia pozycji (`rebalance_session_id`) — norma docelowa

**Intencja produktowa (operator / bot):** przy **ręcznym lub botowym** starcie cyklu (open, reopen po close, swap-before-open) kapitał na ten cykl ma być **wydzielony logicznie**, tak jakby istniało **osobne subkonto portfela** na sesję — z aktualnym stanem tokenów potrzebnym do kolejnego open/reopen, a nie tylko „cały portfel minus inne strategie”.

**To nie jest** osobny keypair on-chain per cykl (multi-wallet w [`MULTI_WALLET_MANUAL_RUNBOOK.md`](MULTI_WALLET_MANUAL_RUNBOOK.md) = inne klucze, nie subkonta sesji).

| Aspekt | Norma docelowa | Stan dziś (faza A + §2 spec) |
| ------ | -------------- | ---------------------------- |
| **Identyfikator cyklu** | `rebalance_session_id` (bot) lub `cost_session_id` (ręczny swap+open) — ten sam UUID w lifecycle + wallet journal | ✅ w lifecycle / częściowo w `wallet_gl_journal_event.cost_session_id` |
| **Inwentarz tokenów sesji** | Po **close**: `returned_a_raw`, `returned_b_raw` (principal + `lp_collected_*`); po **swap**: delty w journal; po **collect**: dopisek fee — **read model sesji** sumuje per mint | ✅ źródło w **lifecycle** (`§6.1` w [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md)); ❌ brak agregatu GL „sesja” |
| **Budżet reopen (USD)** | `T = returned_usd_from_close` z ledgera sesji; bot **nie** downsizuje cicho poniżej `T` | ✅ norma w spec **§2 / §6.1**; wykonanie: executor + pending-open |
| **Co bot „widzi” przed open** | **Preferowane docelowo:** saldo **konta logicznego** `SESSION:{rebalance_session_id}` (per mint, raw/ui). **Fallback do wdrożenia:** `T` z lifecycle + `W` z całego portfela (`effective-balances`) | ❌ tylko `W` globalne + `T` z lifecycle; **policy 3A** — brak twardej rezerwacji SPL |
| **Fee zebrane w trakcie cyklu** | Księgowane na to samo konto logiczne (zwiększają inwentarz sesji) | Częściowo w lifecycle; wallet journal — gdy ścieżka podpięta |
| **Zamknięcie pozycji** | Close + ewentualny collect → aktualizacja konta sesji; kapitał „wraca” z puli (`LP:{pool}`) na `WALLET:{session}` w planie kont | W lifecycle + (częściowo) wallet journal; brak kont LP vs wallet w GL |
| **Izolacja między strategiami** | Inna sesja **nie zużywa** zarezerwowanego inwentarza sesji A bez jawnej polityki | ❌ **§2.1 policy 3A** — wspólny portfel; mitigacja: pending-open, ponawianie |

**Plan kont (faza C — rozszerzenie seed):**

- Konta globalne: `SOL_NATIVE`, `WSOL`, `SPL:{mint}`, `TX_FEE`, …
- Konta pozycji / puli: np. `LP:{position_pubkey}` lub `LP_POOL:{pool}` (liquidity locked).
- **Konta sesji (nowe):** `SESSION:{rebalance_session_id}` per mint — **read model** = suma `confirmed` delt z journalu + lifecycle powiązanych z tym `cost_session_id` / `rebalance_session_id`.
- Przepływ normatywny: **open** (obciążenie LP + odciążenie SESSION), **close** (odwrotnie), **swap** (SESSION mint A ↔ SESSION mint B), **collect** (fee z LP → SESSION).

**Read model (faza D — shadow, potem produkt):**

- Endpoint / pole w API: np. `session_balances` obok `effective-balances` — oznaczone **`source=gl_session_shadow`** dopóki reconcile nie przejdzie.
- Bot / UI przy reopen: **najpierw** `SESSION:*` (ile tokenów na ten cykl), **porównanie** z `T`; `W` globalne tylko jako kontrola / §2.2 (stale read).

**Relacja z policy 3A ([`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md) §2.1):**

- Faza obecna: **bez** escrow / rezerwacji on-chain per sesja.
- Wejście w **konto logiczne SESSION** **nie zastępuje** od razu 3A — najpierw **shadow read model** + telemetria rozbieżności `SESSION` vs `W`.
- **Twarda rezerwacja** (inna strategia nie może ruszyć mintów przypisanych do sesji) = **osobna decyzja produktowa** po fazie D/E; wymaga jawnej zmiany normy §2.1 (wyjście poza 3A).

**Kryteria „done” dla tej normy (przyszły PR):**

1. Plan kont zawiera `SESSION:{uuid}` i reguły delt per `kind` (open/close/swap/collect).
2. Read model Postgres aktualizowany przy każdym `confirmed` z `cost_session_id` / `rebalance_session_id`.
3. Executor / pending-open loguje `session_notional_*` vs `wallet_notional_*` w diagnostyce reopen.
4. UI (opcjonalnie): panel „kapitał sesji” przy stranded / lifecycle.

**keywords:** rebalance_session_id, cost_session_id, session account, logical sub-account, policy-3A, returned_raw, SESSION, chart of accounts, read model

---

## 3. Plan implementacji (fazy)

### Faza A — *Journal + korelacja* ✅ (merytorycznie „jest”)

- [x] JSONL append-only + lock serializujący zapis.
- [x] `pending` / `confirmed` / `failed` + `correlation_id` dla podstawowych flow API.
- [x] GET + UI przeglądu.
- [x] Katalog danych + notatki inżynierskie.

### Faza B — *Kompletność zdarzeń API („każda” operacja portfela przez API)*

**Cel:** każda **operacja wykonywana przez API**, która zmienia stan portfela / pozycji z perspektywy podpisu, generuje wpis (najlepiej ten sam wzorzec pending → wynik).

- [x] `close_position`
- [x] `collect_fees`
- [x] `decrease_liquidity`
- [x] `rebalance_position`
- [ ] Ewentualnie: ścieżki `tx/*` submit jeśli uznacie je za „portfel API”.

**Kryterium ukończenia:** lista endpointów w sekcji „pokrycie” w tym dokumencie + test regresyjny (np. mock `append` / inspekcja `kind` w tailu) dla każdej nowej ścieżki.

### Faza C — *Chart of accounts + spójne delty*

**Cel:** **PostgreSQL:** tabele (lub widoki materializowane) planu kont + reguły w kodzie (Rust): jakie delty zapisujemy dla każdego `kind` + kierunku (np. `SOL_NATIVE`, `WSOL`, `SPL:{mint}`, `TX_FEE_ESTIMATE`, …).

- [ ] Tabele planu kont + konwencja znaku (powiązanie z mint / „kontem logicznym”). **Seed:** `wallet_gl_token_account` / `wallet_gl_curated_pool` (migracja 009) — rozszerzaj przy zmianie listy w `curated_backtest_pools()`.
- [ ] Konta **`SESSION:{rebalance_session_id}`** per mint — norma: [§2.2](#22-konto-logiczne-per-cykl-życia-pozycji-rebalance_session_id--norma-docelowa); powiązanie journal ↔ lifecycle.
- [ ] Walidacja: brak zapisu `confirmed` bez kompletu delt (lub jawny `decode_status`).

### Faza D — *Read model: stan z GL (+ opcjonalnie cache)*

**Cel:** warstwa **„saldo z GL”** w **PostgreSQL** (agregacja / projekcja po `owner` + `mint` lub `account_id`) z:

- [ ] czytelnym **opening balance** (snapshot lub import),
- [ ] **inkrementalną** aktualizacją przy każdym `confirmed` (trigger, worker lub transakcja aplikacji),
- [ ] flagą **stale / needs_reconcile** gdy brakuje zdarzeń lub wykryto lukę.

**Uwaga produktowa:** dopóki Faza B nie jest kompletna, **stan z GL** musi być oznaczony jako **eksperymentalny / shadow** względem RPC.

### Faza E — *Reconcile GL ↔ on-chain*

**Cel:** jednorazowy lub okresowy job / endpoint: porównanie sum GL z odczytem RPC; raport różnic (wyniki sensownie trzymać w **Postgres** lub append-only logu); opcjonalnie „napraw” przez dopisanie korekty (osobny `kind`).

---

## 4. Zasada pracy dla AI / review

1. Zmiany w zachowaniu GL → aktualizuj **ten dokument** (checkboxy + sekcja 2) **oraz** wpis w [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md) z `keywords: wallet_gl, wallet_ledger, …`.
2. Nie rozszerzać „milcząco” zakresu: jeśli coś ma być tylko journal vs pełny read model — **pisać w PR** do której fazy należy.

---

## 5. Changelog dokumentu

| Data | Zmiana |
| ---- | ------ |
| 2026-05-20 | **§2.2:** norma docelowa konta logicznego `SESSION:{rebalance_session_id}` (kapitał cyklu, fee, reopen); relacja z policy 3A i fazami C–D. |
| 2026-05-15 | **Postgres journal:** `wallet_gl_journal_event` (migracja `010_*`), dual-write przy append, odczyt z PG + fallback JSONL. |
| 2026-05-14 | **Postgres seed:** `wallet_gl_token_account` + `wallet_gl_curated_pool` (migracja `009_*`) — konta per mint i pary z `curated_backtest_pools()`. |
| 2026-05-14 | **Decyzja:** docelowy magazyn GL (plan kont, journal, read model sald, reconcile) → **PostgreSQL**; JSONL do migracji. Zaktualizowane opisy Faz C–E. |
| 2026-05-14 | Faza B (część): `close_position`, `collect_fees`, `decrease_liquidity`, `rebalance_position` → wpisy w `wallet-ledger-events.jsonl` (pending→confirmed/failed); `collect_fees` ma delty SPL z różnicy pre/post. |
| 2026-05-13 | Utworzenie dokumentu: wizja księgowa vs faza A; plan B–E. |
