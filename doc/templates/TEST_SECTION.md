# Szablon: sekcja „Testy i kryteria regresji”

Wklej do każdego nowego `IMPLEMENTATION_PLAN_*` / `ROADMAP_*` / `*_PLAN.md` (zasada 7 w [`IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md`](../IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md) §4). Plan bez tej sekcji jest niekompletny. Wypełnij na podstawie mapy fal (§7 tamtego planu).

---

## Testy i kryteria regresji

**Obszar testów (R1):** Ekonomia / GL | Lineage / PnL | Execution / rebalance | Dane / decode | Backtest / strategie | Decision layer / agent | Domain math | API / kontrakt | Web  
**Pozycja w mapie rozwoju:** F_._ (link do §7) — albo „brak, nowa pozycja” + dopisz ją do §7.

| Co chronimy (zachowanie, nie implementacja) | Typ testu | Fixture / dane | Stan |
| ------------------------------------------- | --------- | -------------- | ---- |
| np. „reopen nie zużywa tokenów spoza SESSION” | golden / niezmiennik / kontrakt / unit / db / manual | np. `crates/.../tests/fixtures/...` (syntetyczny, w repo) | ❌ / 🟡 / ✅ |

**Test exit gate:** warunek zamknięcia planu — które testy z tabeli muszą być zielone w CI (job `rust` / `web` / `db`).

**Bugi powiązane:** `BUG-...` — test wpisany w `Guards/tests` w [`BUGS.md`](../BUGS.md).

**Golden delta:** czy zmiana ma ruszać istniejące goldeny (PnL / fees / salda / sizing)? Jeśli tak — które i dlaczego (to samo w opisie PR).

**Poza automatem:** co zostaje ręczne (devnet `#[ignore]`, E2E operatora) i dlaczego.

Zasady dla testów w tej sekcji: hermetyczne (bez sieci, bez repo `data/`, env przez `test_env::EnvGuard`), bez pustych passów — `.cursor/rules/test-integrity.mdc`. Po wdrożeniu testów zaktualizuj katalog [`TESTS.md`](../TESTS.md) (pkt 8 tej reguły).
