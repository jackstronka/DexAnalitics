# Devnet bot production readiness (MVP -> production-like)

## Cel

Przejsc z "devnet smoke / symulacja" do "produkcyjnego bota na devnecie":
- deterministyczny lifecycle tx (`open -> decrease -> collect -> close`),
- bezpieczny model podpisu (keypair server-side lub Phantom client-side),
- operacyjna niezawodnosc (retry, idempotency, monitoring, runbook).

## Faza 1 (must-have) - blokery GO-LIVE na devnecie

- [ ] **Realne buildy tx, bez placeholdera**  
  `POST /tx/*/build` ma teraz realne instrukcje Whirlpool dla `open` (przez `orca_whirlpools` SDK) zamiast placeholdera; docelowo dopinamy analogicznie `decrease/collect/close` i pełną listę wymaganych kont.

- [ ] **Silny policy gate + guardrails ryzyka**  
  Dla build/submit wymagane: allowlist programow, walidacja kont docelowych, limity slippage/min-out, limity kwot i sanity-check tick range.

- [ ] **Fail-fast konfiguracji testow i bota**  
  Brak `KEYPAIR_PATH`/`SOLANA_KEYPAIR_PATH`, brak `SOLANA_RPC_URL`, brak krytycznych env -> twardy fail (bez cichego przechodzenia).
- Weryfikacja na `devnet_ -- --ignored` (bieżący kod): testy `devnet_*lifecycle_keypair_smoke` oraz `devnet_*unsigned_tx*` failują z powodu braku `KEYPAIR_PATH` / `SOLANA_KEYPAIR_PATH` (to jest oczekiwane po naszym hardeningu).

- [ ] **Walidacja skutkow on-chain po lifecycle**  
  E2E po kazdym kroku sprawdza efekt on-chain (position/liquidity/token balances/fees), nie tylko status wywolania.

- [ ] **Idempotency + retry strategy**  
  Każda operacja ma `correlation_id`, retry z backoff i limitem prob, jasna obsluga duplicate submit/restart.

- [ ] **Negatywne E2E scenariusze krytyczne**  
  Co najmniej: brak srodkow, zly pool/position, signature mismatch, policy gate reject, simulation failure.

## Faza 2 (should-have) - stabilnosc operacyjna

- [ ] **Potwierdzenia transakcji i finality policy**  
  Rozdzielic `submitted` vs `confirmed` vs `finalized`; retry/reconcile tylko na podstawie ustalonej polityki.

- [ ] **Recovery po partial failure**  
  Jesli `open` przeszedl, a `decrease/collect/close` nie - bot ma workflow naprawczy i cleanup.

- [ ] **Lepsza observability i alerting**  
  Metryki: success/fail rate tx, simulate fail reasons, latency p95, rate limity RPC, DLQ/event bus health.

- [ ] **Runbook day-2 operations**  
  Procedury start/stop/restart, rotacja kluczy, incident handling, rollback, GO/NO-GO checklist przed runem.

- [ ] **Rate-limit aware RPC strategy**  
  Failover endpointow, budzet zapytan, timeout profile, kontrola inflight i degradacja "safe mode".

## Faza 3 (nice-to-have) - twardnienie produkcyjne

- [ ] **Canary mode / progressive rollout**  
  Najpierw 1 pool i niski kapital, potem stopniowe rozszerzanie po metrykach SLO.

- [ ] **Automatyczne raporty jakosci decyzji**  
  Raport dzienny: powod decyzji, tx outcome, rozjazd miedzy symulacja i wykonaniem.

- [ ] **Test matrix wielu pul/protokolow**  
  Orca jako baza, potem scenariusze z roznymi profilami zmiennosci i plynnosci.

- [ ] **Formalne SLO/SLI**  
  Np. `>= 95%` successful submissions i `<= 2%` simulation false-positives w oknie 24h.

## Definition of Ready (Devnet Production-Like)

Mozna uznac bota za "production-like on devnet", gdy:
- wszystkie pozycje z Fazy 1 sa odhaczone,
- wszystkie testy devnet E2E (pozytywne + negatywne) przechodza stabilnie przez min. 3 kolejne runy,
- istnieje runbook operacyjny i monitoring z alertami,
- jest jasna procedura rollback/recovery.

## Szybki plan realizacji (kolejnosc)

1. Domknac realne buildy tx i guardrails (Faza 1).
2. Rozszerzyc E2E o twarda walidacje skutkow on-chain i scenariusze negatywne (Faza 1).
3. Dodac idempotency/retry/finality policy (Faza 1 -> Faza 2).
4. Domknac monitoring + runbook operacyjny (Faza 2).
5. Uruchomic canary mode i SLO-based rollout (Faza 3).
