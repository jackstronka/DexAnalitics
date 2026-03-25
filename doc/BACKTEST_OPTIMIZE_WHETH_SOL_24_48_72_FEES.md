# backtest-optimize — scalone okna 24h / 48h / 72h

**Para:** whETH / SOL  
**Pool:** `HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF`  

**Cel (ranking siatki, pełna siatka, wszystkie strategie):** `--objective fees` — maksymalizacja **`total_fees`** w modelu (snapshoty).  
*(Jeśli w szablonie było `vs_hodl`, to inny ranking; pod „jak najlepsze fees” używamy właśnie **`fees`**.)*  

**Kapitał:** $7000 · **Tx:** $0.10 · **`--windows`:** 1  

**Dane:** ścieżka cen i opłaty ze **snapshotów Orca** (`--price-path-source snapshots`, `--fee-source snapshots`, `--fee-swap-decode-status ok`).  
**Udział LP:** `--lp-share 0.0001` (mały udział → małe kwoty USD fee w tabeli, ale **kolejność** rankingu jest spójna z modelem).

**Siatka:** pełna (`--all-rows` / `--full-ranking`) — wszystkie strategie domyślne: `static`, `oor_recenter`, `threshold_*`, `periodic_12h/24h/48h/72h`, `il_limit_*`, `retouch_shift`; szerokości z `--min-range-pct` / `--max-range-pct` / `--range-steps` (domyślnie 1%–15%, 10 kroków).

## Uruchomienie (PowerShell, repo root)

```powershell
.\scripts\export_optimize_merged_24_48_72_fees_snapshots.ps1
```

Równoważnie:

```powershell
.\scripts\export_optimize_merged_24_48_72_full.ps1 -Objective fees
```

## Wynik (jeden plik tekstowy)

`data/snapshot_logs/optimize_tables_merged_24_48_72_fees.txt` — trzy bloki: **WINDOW LAST 24h**, **48h**, **72h**, każdy z pełną tabelą rankingu.

**Ostatni wygenerowany eksport (przykład):** nagłówek pliku zawiera `Generated:` oraz `Objective: fees`.  
Skrót **BEST** z tego runu (snapshoty, `lp-share 0.0001`):  
- **24h:** width ~4.11 %, static, Score (fees) ~2.31 USD, TIR 100 %  
- **48h:** width ~4.11 %, static, Score ~3.09 USD  
- **72h:** width ~4.11 %, static, Score ~3.71 USD  

*(Kwoty fee są niskie przez mały `--lp-share`; ważna jest **relacja** między wierszami siatki.)*

### Dlaczego w tabeli „wszystko wygląda tak samo” przy `fees`

Dla **tego samego** zakresu (Lower/Upper) strategie, które **w ogóle nie robią rebalance’u** w symulacji, przechodzą **tę samą ścieżkę** → **`total_fees` i kolumna Score są identyczne**. To nie jest błąd — to **remis** na pierwszym kryterium.

Od wersji CLI z tie-breakerami: przy remisie fee ranking sortuje dalej wg **lepszego vs HODL**, potem **mniej rebalance’ów**, potem nazwy strategii. Nadal możesz widzieć wiele wierszy z tym samym Score/Fees — to **redundantne wiersze** (te same opłaty, wybór strategii bez znaczenia, dopóki nie ma rebalansu).

Żeby tabela była „bogatsza” w różnice: dłuższy horyzont, inny `--lp-share`, strategie które faktycznie rebalance’ują, albo **`--objective composite`**.

---

*Uwaga:* Dla celu „blisko HODL” użyj `--objective vs-hodl`. Ten dokument i plik `.txt` opisują wariant **fees**.
