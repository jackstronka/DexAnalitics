# Snapshot Issues Playbook (symptomy -> gotowe rozwiązania)

Cel: mieć szybki zestaw “co było nie tak” przy pipeline’ach snapshotów/fee oraz “co zrobić” bez wchodzenia w analizę od zera.

Dokument jest celowo w stylu operacyjnym: **symptom w logach** → **co to zwykle znaczy** → **szybkie kroki naprawcze** → **jak zweryfikować**.

---

## Szybka ścieżka diagnostyczna (zanim zaczniesz cokolwiek naprawiać)

1. Otwórz pliki wejściowe:
   - `data/pool-snapshots/<protocol>/<pool>/snapshots.jsonl`
   - opcjonalnie `data/pool-snapshots/<protocol>/<pool>/snapshots.jsonl.repaired`
2. Uruchom:
   - `clmm-lp-cli data-health-check --max-age-minutes 30 --min-decode-ok-pct 65`
3. Jeśli problem dotyczy konkretnej puli:
   - `clmm-lp-cli snapshot-readiness --protocol <orca|raydium|meteora> --pool-address <POOL_ADDRESS>`

Wynik: jeśli readiness nie jest “OK”, to najpierw napraw dane (snapshoty), dopiero potem interpretuj backtest.

---

## 1) “Za mało snapshotów w oknie” / tylko 1 krok / wyniki jakby puste

### Symptom (log)
- `Loaded 1 snapshot steps ...`
- tabele wyników mają te same wiersze, `Rebalances: 0`, czasem `Fees: $0` przy `--fee-source snapshots`

### Najczęstsza przyczyna
- Okno czasowe nie pokrywa timestampów z `ts_utc` w `snapshots.jsonl`.
- `--hours 24` bywa kłopotliwe dla Twoich lokalnych danych (różne okna kolekcji / timezone / `end_ts` jako “exclusive” w kodzie).

### Gotowe rozwiązanie
1. Zamiast `--hours 24` przejdź na deterministyczne daty:
   - `--start-date YYYY-MM-DD --end-date YYYY-MM-DD`
2. Dobierz daty tak, żeby trafić w zakres `ts_utc` istniejący w pliku:
   - opcjonalnie: ustaw `--start-date` i `--end-date` na datę z którą snapshot jest publikowany lokalnie (u Ciebie często kończy się na datach z 25.03).

### Jak zweryfikować
- Przy ponownym uruchomieniu powinno być **dużo więcej niż 1 step**, np. `Loaded ~100+ snapshot steps ...`.

---

## 2) `snapshot fee index unavailable/empty; fees may fall back to candles.`

### Symptom (log)
- `⚠️ backtest-optimize: snapshot fee index unavailable/empty; fees may fall back to candles.`
- w tabeli: `Fees` bardzo małe lub równe `0.00` mimo, że `Loaded X snapshot steps`

### Co to zwykle znaczy
- Pipeline Tier 2 dla snapshot fee proxy nie ma wystarczających pól / danych wejściowych.
- Najczęściej braki w polach fee-growth/counters albo “przerwane”/brudne rekordy w JSONL.

### Gotowe rozwiązanie
1. Sprawdź readiness dla konkretnej puli:
   - `clmm-lp-cli snapshot-readiness --protocol <orca|raydium|meteora> --pool-address <POOL_ADDRESS>`
2. Napraw zależnie od protokołu (w snapshot JSONL):
   - **Orca (Tier 2)**: potrzebne konsekwentnie co najmniej:
     - `fee_growth_global_a` i `fee_growth_global_b`
     - albo `protocol_fee_owed_a` i `protocol_fee_owed_b`
     - oraz do alokacji share: `liquidity_active` (w trybie snapshot-only modelu)
   - **Raydium (Tier 2)**: potrzebne konsekwentnie:
     - `fee_growth_global_a_x64` i `fee_growth_global_b_x64`
     - albo `protocol_fees_token_a` i `protocol_fees_token_b`
     - oraz `liquidity_active`
   - **Meteora (Tier 2)**: potrzebne:
     - `protocol_fee_amount_a` i `protocol_fee_amount_b`
     - (plus wspólne pola do USD/TVL i share w zależności od trybu)
3. Jeśli podejrzewasz brudne/niepełne linie:
   - upewnij się, że istnieje `snapshots.jsonl.repaired` i użyj go (kod preferuje repaired, jeśli istnieje).

### Jak zweryfikować
- Po poprawce warning zniknie (albo `per-step fees` będą liczone).
- W output `Fees:` powinny być > 0.00 w scenariuszach z realnym fee accrual.

---

## 3) Meteora snapshot-only: brak vaultów => prośba o `--lp-share`

### Symptom (log)
- `Meteora snapshot-only: set --lp-share, or re-run meteora-snapshot-curated ... needed for TVL → lp_share`

### Najczęstsza przyczyna
- Snapshoty Meteora są “stare” albo przygotowane bez pól `vault_amount_a` / `vault_amount_b`.

### Gotowe rozwiązanie
1. “Szybko działa”: uruchom backtest-optimize z ręcznym share:
   - `--lp-share <fraction>`, np. `0.0001` albo wartość która odpowiada Twojej rzeczywistej ekspozycji (0..1)
2. “Docelowo”: zregeneruj snapshoty Meteora w trybie, który zapisuje vaulty:
   - uruchom `meteora-snapshot-curated` (tak jak w komunikacie błędu).

### Jak zweryfikować
- `backtest-optimize` nie powinien już przerywać się tym komunikatem i `lp_share` będzie wyliczane (lub przynajmniej nie blokuje uruchomienia).

---

## 4) `--lp-share 1.0` (override) może sztucznie napompować PnL

### Symptom (log)
- PnL/Fees w output wydają się nienaturalnie wysokie.
- Często dodatkowo widzisz dużo `Fees` mimo braków/małej liczby rebalansów (albo “wszystko wygrało” tym samym zakresem).

### Co to zwykle znaczy
- W snapshot-only trybie `--lp-share` nadpisuje model udziału LP w puli.
- Jeśli w praktyce Twoja pozycja nie odpowiada 100% puli, override powoduje przeszacowanie “effective share” i mnoży fee-accual w symulacji.

### Gotowe rozwiązanie
1. Przy snapshotach, które mają vaulty (np. nowsze Orca/Raydium albo Meteora z vaultami):
   - usuń override i pozwól liczyć `lp_share` z TVL proxy:
     - uruchom bez `--lp-share`
2. Jeśli masz tylko stare Meteora snapshoty (brak vaultów):
   - override jest wymagany, ale ustaw wartość sensownie (0..1), a nie “1.0 na ślepo”.
   - docelowo: zregeneruj snapshoty tak, żeby zawierały `vault_amount_a/vault_amount_b`.

### Jak zweryfikować
- Porównaj 2 uruchomienia na tym samym oknie i tym samym protokole:
  - (a) bez override (jeśli się da)
  - (b) z override
- Jeśli wynik (PnL/Fees) skacze o rząd wielkości, to znaczy, że override był źle skalibrowany.

---

## 5) Snapshot-only USD conversion robi wywołania zewnętrzne (Dexscreener) / wymaga API

### Symptom
- w logach/observacji widzisz wywołania do Dexscreener albo problemy z quote USD

### Co to zwykle znaczy
- Token B (quote) nie jest USDC ani USDT.
- Kod mapuje USDC/USDT do `1.0`, a dla innych mintów używa Dexscreener.

### Gotowe rozwiązanie
1. Do porównań “snapshot-only bez usług USD” używaj quote:
   - USDC mint: `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v` (ustaw jako `mint-b`)
   - USDT mint: `Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB` (ustaw jako `mint-b`)
2. Jeśli chcesz etykietę “USD”, ale bez wywołań zewnętrznych:
   - używaj w praktyce `SOL/USDC` lub `SOL/USDT`, a w output traktuj to jak “SOL/USD”.

### Jak zweryfikować
- `backtest-optimize` kończy się bez potrzeby API / bez logów o problemach z Dexscreener.

---

## 6) `decode_status=partial` masowo / swapy prawie nie pokrywają fee (zwykle jako efekt uboczny fallbacków)

### Symptom
- w raportach `swaps-decode-audit` dominuje `partial` / timeout
- w backtestach (gdy fee model wpada w inne źródła) widzisz podejrzanie niskie `Fees`

### Najczęstsze przyczyny (z historii repo)
1. **Root cause (konkretny bug 2026-03-19)**:
   - `meta` czytane z złej ścieżki (`transaction.meta` zamiast top-level `meta` po serde flatten)
   - brak dołożenia v0 ALT kont z `meta.loadedAddresses`
2. Endpoint RPC rate-limit / brak historii na danym RPC (timeouty mimo limitów)

### Gotowe rozwiązanie
1. Napraw decode i zbuduj poprawne local decoded:
   - `swaps-enrich-curated-all --refresh-decoded --max-decode <large-enough> ...`
2. Zwiększ stabilność RPC:
   - ustaw `SOLANA_RPC_URL` na endpoint który daje sensowne `getTransaction`
3. Zrób audit:
   - `swaps-decode-audit --save-report`

### Jak zweryfikować
- W `data/reports/decode_audit_*.json` widać wzrost `% decode_status=ok` / `ok_*`.

---

## 7) `snapshots.jsonl` ma brudne/niepełne linie JSON (collector przerwał w trakcie)

### Symptom
- brak pewnych snapshotów albo spadek “ilości stepów”
- niestabilność readiness

### Gotowe rozwiązanie
1. Szukaj pliku naprawczego:
   - preferowany: `snapshots.jsonl.repaired`
2. Jeśli repaired nie istnieje:
   - uruchom ponownie snapshot collector dla puli (albo napraw pipeline generując repaired).

### Jak zweryfikować
- readiness przestaje “skakać”
- `Loaded X snapshot steps` rośnie do stabilnych wartości.

---

## Mini-checklist przed uruchomieniem “porównaj protokoły” (snapshot-only)

1. Czy puste wyniki nie wynikają z okna czasu?
   - użyj `--start-date/--end-date`, a nie `--hours 24`, jeśli nie masz pewności pokrycia.
2. Czy Tier 2 readiness jest OK dla każdej puli?
   - `snapshot-readiness --protocol ... --pool-address ...`
3. Czy quote USD jest stable?
   - USDC/USDT mint => bez Dexscreener.
4. Czy Meteora ma vaulty?
   - jak nie ma: `--lp-share` albo regeneracja curated.
5. Czy widzisz warning “snapshot fee index unavailable”?
   - jeśli tak, napraw snapshot pola fee proxy (pkt 2).

