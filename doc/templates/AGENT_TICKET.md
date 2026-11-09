# Szablon: ticket dla agenta (E3 / G7)

Wklej na początek promptu / opisu PR / issue, zanim agent ruszy kod.  
Źródło: [`IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md`](../IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md) (E3).  
Polityka: [`.cursor/rules/test-integrity.mdc`](../../.cursor/rules/test-integrity.mdc), [`AI_MERGE_CHECKLIST.md`](../AI_MERGE_CHECKLIST.md).  
Pad CI → [`CI_TRIAGE.md`](CI_TRIAGE.md) (D4). Nowa sekcja testów w planie → [`TEST_SECTION.md`](TEST_SECTION.md).

**Cel:** jeden ticket = jeden ID z planu, jasny zakres, jawne zakazy, komenda verify — bez „napraw wszystko” i bez cichej edycji goldenów.

---

## 1. Nagłówek (obowiązkowy)

| Pole | Wartość |
| ---- | ------- |
| **Plan ID** | np. `B3`, `C2`, `E1` — z tabeli w `IMPLEMENTATION_PLAN_*` / `MASTER` |
| **Tytuł (1 linia)** | |
| **Obszar testów (R1)** | Ekonomia / GL \| Lineage / PnL \| Execution / rebalance \| Dane / decode \| Backtest / strategie \| Decision layer / agent \| Domain math \| API / kontrakt \| Web |
| **Done when** | konkretne kryterium z planu (nie „działa”) |
| **Out of scope** | co **nie** wchodzi w ten ticket |
| **Wymaga GO przed…** | np. edycją `.snap` / expected money / OpenAPI / migracją |

---

## 2. Zakazane pliki / akcje (domyślnie — skreśl tylko świadomie)

Agent **nie** rusza bez jawnego GO w ticketcie / odpowiedzi użytkownika:

| Zakaz | Dlaczego |
| ----- | -------- |
| `*.snap`, `**/tests/fixtures/**` (expected money), `**/snapshots/**` | `economic_regression` — najpierw raport było/jest/Δ |
| `crates/api/openapi.json` + ręczna edycja `web/src/lib/api.gen.ts` | kontrakt C1/C2 — tylko `make openapi` + `npm run gen:api` po GO |
| `.env`, keystore, sekrety, webhook URL | nigdy w git |
| Osłabianie asercji: `is_ok()`, szersza tolerancja, skasowany assert, nowy `#[ignore]` / `it.skip`, wczesny `return` bez assert | `assertion_weakened` (D3 Bugbot) |
| Retry / sleep „żeby przeszło”, sieć poza localhost mock, `set_current_dir`, zapis do repo `data/` w teście | hermetyczność |
| Push na `main`, `--no-verify`, force push bez prośby | G1 / A10 |

**Dozwolone bez osobnego GO:** testy nowe/rozszerzone w zakresie ticketu, docs planu/`TESTS.md`/`ENGINEERING_NOTES` powiązane z ID, skrypty CI/tools jeśli ticket to przewiduje, `stale_test` / `infra` / `non_hermetic` per D4.

---

## 3. Komenda verify (wpisz jedną — agent odpala tę)

| Kontekst | Komenda |
| -------- | ------- |
| Pełny gate (jak CI / pre-push) | `make verify` (Windows: `tools/verify.ps1`) |
| Szybki lokalny (E1) | `make verify-fast` / `tools/verify-fast.ps1` |
| Wąski crate | `cargo nextest run -p <crate> <filtr>` |
| Web only | `cd web && npx tsc --noEmit && npm test` |
| DB | job/`CLMM_REQUIRE_DB_TESTS=1` — nie skipuj |

**Verify dla tego ticketu:** `________________`

Po zmianie golden/fixture/`openapi.json`: w body PR sekcja **Golden delta:** (D1) + świadomość CODEOWNERS (D2).

---

## 4. Characterization najpierw (PnL / GL / backtest / sizing / lineage)

Gdy ticket dotyka **pieniędzy lub łańcucha pozycji**, kolejność jest sztywna:

1. **Zidentyfikuj istniejący golden / niezmiennik / fixture** w obszarze (katalog: [`TESTS.md`](../TESTS.md); krytyczne ścieżki: `scripts/ci/critical-area-test-gate.sh`).
2. **Uruchom characterization** — ten sam test / wąski filtr — **przed** zmianą semantyki. Zanotuj PASS/FAIL i kluczowe liczby.
3. **Zmiana produktu** tylko po baseline; jeśli golden się rozjeżdża → klasa `economic_regression`: tabela było/jest/Δ, opcjonalnie `BUGS.md`, **STOP** na expected aż do GO (+ sekcja Golden delta w PR).
4. **Nowy test w tym samym PR** gdy brakuje ochrony (mapa fal §7 planu regresji); zaktualizuj `TESTS.md` gdy nowy plik / golden / harness.

Bez characterization na ścieżce money: ticket traktuj jako niepełny — nie „najpierw refactor, test później”.

---

## 5. Prompt do wklejenia (wypełnij i wyślij agentowi)

```text
Ticket E3
Plan ID: <ID>
Tytuł: <…>
Obszar (R1): <…>
Done when: <…>
Out of scope: <…>

Zakazane bez GO: *.snap / fixtures expected money / openapi ręczne / assertion_weakened / .env
Verify: <make verify | make verify-fast | cargo nextest …>

Characterization (jeśli PnL/GL/backtest/sizing/lineage):
- istniejący test/fixture: <ścieżka lub „brak — dodaj w tym PR”>
- odpal przed edycją semantyki: <komenda>

Zrób tylko ten ID. Analysis first; implementacja po GO jeśli bugfix.
Czerwone CI → doc/templates/CI_TRIAGE.md.
```

---

## 6. Checklista zamknięcia ticketu

- [ ] Plan ID w tytule PR / pierwszej linii odpowiedzi
- [ ] Diff nie zawiera zakazanych plików bez GO + Golden delta
- [ ] Verify z §3 zielone lokalnie (ta sama komenda co w ticketcie)
- [ ] Przy money: characterization odpalona przed zmianą expected
- [ ] Nowy test / `TESTS.md` / `ENGINEERING_NOTES` (`keywords:`) gdy behawior/API materialnie zmienione
- [ ] `AI_MERGE_CHECKLIST.md` przejrzany przed „gotowe”
