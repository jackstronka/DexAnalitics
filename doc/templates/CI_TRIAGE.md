# Szablon: triage pada CI (D4 / G6)

Dla agenta Cursor / Cloud (`@cursor`) przy czerwonym checku na PR.  
Źródło klas: [`IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md`](../IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md) §3–§4.  
Polityka: [`.cursor/rules/test-integrity.mdc`](../../.cursor/rules/test-integrity.mdc).

**Cel:** naprawić to, co wolno — albo **zatrzymać się** i oddać decyzję człowiekowi. Nie „zazieleniać” CI przez osłabienie testu.

---

## 1. Zbierz kontekst (przed jakąkolwiek edycją)

Wklej / uzupełnij:

| Pole | Wartość |
| ---- | ------- |
| PR / gałąź | |
| Job CI (`rust` / `web` / `db` / `quality_gates` / …) | |
| Nazwa testu / kroku | |
| Komenda lokalna do reprodukcji | np. `cargo test -p … -- --nocapture` |
| Fragment logu (≤20 linii) | |
| Czy pad jest tylko w CI (lokalnie zielony)? | tak / nie |

---

## 2. Klasyfikacja (obowiązkowa — jedna klasa)

Wybierz **jedną** klasę i uzasadnij 1–2 zdaniami (dowód z logu / diffu):

| Klasa | Kiedy | Co wolno agentowi | Czego **nie** wolno |
| ----- | ----- | ----------------- | ------------------- |
| `stale_test` | test nie kompiluje / nie pasuje do rename/pola; semantyka produktu bez zmian | dopasować test do nowego API **bez** zmiany tego, co asercja gwarantuje | zmieniać expected money, kasować aserty, `#[ignore]` |
| `flake` / `non_hermetic` | race env, sieć, repo `data/`, zegar ścienny, pass tylko lokalnie | izolacja: `test_env::EnvGuard`, tempdir, injected clock, localhost mock | retry, szersze okna czasu, skip „na razie” |
| `infra` | brak skryptu, zły workflow, path, tool | naprawić CI / skrypt / ścieżkę | ukryć pad przez `#[ignore]` w produkcie |
| `economic_regression` | diff golden PnL/IL/fees/sald/sizing/lineage albo padnięty niezmiennik | **tylko** raport (tabela było/jest/Δ) + opcjonalnie wpis [`BUGS.md`](../BUGS.md); **STOP** — czekaj na GO | edytować `.snap` / expected / fixture money, `INSTA_UPDATE`, poluzować tolerancję |
| `assertion_weakened` | ktoś już usunął/osłabił asercję, dodał `#[ignore]`, pusty `return` | **odrzucić** zmianę / przywrócić siłę asercji; Bugbot (D3) | merge „żeby było zielono” |

Jeśli niepewne: napisz `unknown yet`, wymień wykluczone klasy i kolejny krok diagnostyczny — **bez** edycji expected.

---

## 3. Raport wyjściowy (wklej do PR / odpowiedzi użytkownikowi)

```text
## CI triage (D4)

Class: <stale_test | non_hermetic | infra | economic_regression | assertion_weakened | unknown yet>
Evidence: <job + test + 1 fakt z logu/diffu>
Allowed action: <co zrobię / STOP>
Next: <komenda verify albo pytanie do człowieka>
```

Przy `economic_regression` dodaj mini-tabelę (albo wynik `python tools/golden_delta.py --git-base origin/main`):

| Metryka | Było | Jest | Δ |
| ------- | ---- | ---- | - |
| | | | |

I **nie** commituj zmian snapshotów, dopóki użytkownik nie da jawnego GO + sekcja **Golden delta:** w opisie PR.

---

## 4. Checklista przed push / re-run CI

- [ ] Klasa podana w odpowiedzi / komentarzu PR
- [ ] Żaden assert nie zamieniony na `is_ok()` / szerszą tolerancję / usunięty
- [ ] Brak nowego `#[ignore]` / `it.skip` „żeby przeszło” (wyjątek: nowy job `manual:` na prawdziwych danych)
- [ ] Brak wczesnego `return` w teście bez asercji
- [ ] Przy zmianie fixture/snap/OpenAPI: sekcja **Golden delta:** w body PR (D1) + świadomość CODEOWNERS (D2)
- [ ] Lokalnie: ta sama komenda co padający job (albo `make verify` / `tools/verify.ps1`)

---

## 5. Szybkie mapowanie job → typowa klasa

| Objaw | Startowa klasa |
| ----- | -------------- |
| błąd kompilacji testu / brakujące pole w fixture | `stale_test` |
| pad tylko pod `unshare -n` / flaky env | `non_hermetic` |
| `golden_delta` bez sekcji w PR | `infra` (brak sekcji) albo proces D1 — **nie** edytuj snapów „na ślepo” |
| diff liczb w `.snap` / pad proptest na PnL/IL | `economic_regression` |
| Bugbot: `assertion_weakened` | `assertion_weakened` |
| brak `scripts/ci/...`, zły workflow | `infra` |
