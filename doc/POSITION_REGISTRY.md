# Rejestr otwartych / zamkniętych pozycji (`registry.jsonl`)

## Po co

- **Jedno miejsce** (append-only JSONL), które mówi: ta pozycja została **otwarta** (`registry_open`) albo **zamknięta** (`registry_close`).
- **Kolektory** (skrypty snapshotów, checkpointy per-pozycja, alerty) mogą:
  - wczytać plik i wyliczyć **aktualnie otwarte** pozycje;
  - po wierszu `registry_close` **przestać** zbierać dane „pod tę pozycję” (lub wyłączyć filtry zawierające ten `position_pubkey`).

To **nie zastępuje** ledgera kosztów (`orca_position_lifecycle.jsonl`) ani IL/fee JSONL — uzupełnia je o **stan życia pozycji** pod automatyzację.

## Plik i zmienne

| | |
|---|---|
| Domyślna ścieżka | `data/positions/registry.jsonl` |
| Nadpisanie | `CLMM_POSITION_REGISTRY_PATH` |
| Korelacja z innymi ledgerami | opcjonalnie `CLMM_REBALANCE_SESSION_ID` (pole `rebalance_session_id` w wierszu) |

## Kto dopisuje

- **`orca-position-open`** (CLI) → `registry_open`, `source=cli`
- **`orca-position-close`** (CLI) → `registry_close`, `source=cli`
- **`RebalanceExecutor`** (open / full-range open / close w rebalansie) → `source=orca_bot`

## Schemat wiersza (schema_version = 1)

- `event`: `registry_open` \| `registry_close`
- `position_pubkey`, `pool_address`, `owner_pubkey`, `signature`
- `source`: `cli` \| `orca_bot`
- `rebalance_session_id` (opcjonalnie)
- `rpc_url`, `accounting_note`

## Jak wyliczyć „otwarte teraz”

Append-only: dla każdego `position_pubkey` weź **ostatni** wiersz (po czasie / kolejności w pliku). Jeśli ostatni to `registry_open` → pozycja uznawana za otwartą; jeśli `registry_close` → zamknięta.

Proste podejście operacyjne: `grep` / skrypt, który buduje mapę `position → last_event`.

## Pula Whirlpool vs PDA pozycji (częsta pomyłka przy `close`)

- Konto **puli** (`whirlpool`) ma **653 bajty** danych (to nie jest adres pozycji).
- Konto **pozycji** (PDA programu Whirlpool) ma **216 bajtów** — to **ten** adres podajesz do `orca-position-close` i do bota.
- Przy **`OpenPositionWithTokenExtensions`** w Solscan: w pierwszej instrukcji Whirlpool **3. konto = `position`**, **6. = `whirlpool`** (pool). Łatwo skopiować odwrotnie.
- Rozszerzenia Token-2022 dotyczą **mintu NFT** pozycji, nie rozmiaru konta pozycji — layout pozycji pozostaje 216 B.

## Retro: pierwsza pozycja mainnet bez wpisu

Wpisy powstają **od momentu wdrożenia** tej funkcji. Starsze pozycje możesz **jednorazowo** dopisać ręcznie jednym wierszem `registry_open` (z prawdziwym `signature` open i adresem pozycji) albo zostawić tylko ledger kosztów — rejestr jest od teraz źródłem „open/close” dla kolektorów.
