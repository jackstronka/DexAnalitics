# Plan wdrożenia live: Aerodrome Slipstream (Base), fee z handlu

Dokument opisuje **jak dojść od zera do bezpiecznego live** dla integracji **Aerodrome Slipstream** na **Base** przy założeniu z repozytorium: **tylko unstaked LP** — **fee ze swapów** (NPM + pula), **bez** stake w gauge i **bez** emisji AERO w pierwszym zakresie.

## 0. Zasada: najpierw komunikacja, potem cięższe rzeczy

**Komunikacja = kontrakt i przewidywalność**, nie „maksymalna liczba funkcji on-chain”.

1. **Najpierw:** stabilna ścieżka **klient → API → Base RPC** — ścieżki REST, **OpenAPI**, kody błędów (np. brak `BASE_RPC_URL` → 503, błąd węzła → 502), timeouty, ewentualnie logowanie po stronie API (bez sekretów). Ustalenie, **kto** woła API (bot, dashboard, skrypty) i z jakim **bazowym URL** / retry.
2. **Potem:** rozszerzanie **read modelu** (`liquidity`, `token0`/`token1`, itd.) — nadal bez podpisywania tx.
3. **Na końcu:** symulacje, **Quoter**, budowa tx, automatyzacja strategii — bo to już zależy od poprawnej i zaufanej warstwy z pkt. 1–2.

Istniejący endpoint `GET …/slot0` jest **przykładem pkt. 1** (cienki read przez ustalony kontrakt HTTP), a nie zamknięciem całej integracji.

**Kod referencyjny (adresy Gauges V3):** `crates/protocols/src/aerodrome_slipstream/mod.rs`.

**API (read-only):** `GET /api/v1/evm/base/aerodrome-slipstream/pools/{pool}/slot0` — wymaga `BASE_RPC_URL` (Base JSON-RPC).

---

## Referencyjne pary (UI Aerodrome — Wasze screeny)

| Para (UI) | Badge | Uwaga z UI |
|-----------|--------|------------|
| **WETH – cbBTC** | `CL100` | wartość **~0,023%** (w UI może to być bieżący fee / APR / inna metryka — **nie** traktujcie jej jako sztywnego „fee tier” bez odczytu z puli). |
| **WETH – USDC** | `CL100` | **~0,0598%** — j.w. |
| **USDC – cbBTC** | `CL100` | **~0,0297%** — j.w. |

**Interpretacja `CL100`:** w ekosystemie Aerodrome/Slipstream oznacza to zwykle pulę **concentrated liquidity** z **tick spacing = 100** (zgodnie z tabelą spacing ↔ fee w upstream `SPECIFICATION.md`; dokładny fee i moduły i tak bierzecie **on-chain**).

**Tokeny na Base (do `getPool` / weryfikacji `token0`/`token1`)** — typowe adresy mainnet; zawsze zweryfikujcie na [BaseScan](https://basescan.org):

| Token | Adres (Base mainnet) |
|-------|----------------------|
| WETH | `0x4200000000000000000000000000000000000006` |
| USDC | `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` |
| cbBTC | `0xcbb7c0000ab88b473b1f5afd9ef808440eed33bf` |

**Adres kontraktu puli:** UI zwykle linkuje do BaseScan albo można wyliczyć `getPool(token0, token1, tickSpacing)` na właściwej `PoolFactory` — na pierwszy live **najbezpieczniej** skopiować **adres puli** z explorer’a i trzymać go w allowliście.

---

## 1. Oficjalna dokumentacja i „jak to się robi”

| Źródło | Po co |
|--------|--------|
| [aerodrome-finance/slipstream](https://github.com/aerodrome-finance/slipstream) | **Kontrakty**, `README.md` (**tabele deployów** na Base), `SPECIFICATION.md` (fee, gauge, unstaked fee module, zachowanie względem Uniswap v3). |
| [aerodrome.finance/docs](https://aerodrome.finance/docs) | Tokenomika, epoki, rola veAERO — **nie** zastępuje integracji technicznej, ale jest potrzebna kontekstowo (np. czemu istnieją gauge’e). |
| [Base docs — network](https://docs.base.org/) | Chain ID **8453**, RPC, mosty, podstawy L2. |
| [Uniswap v3 Core / Periphery](https://docs.uniswap.org/contracts/v3/overview) | Slipstream zachowuje **interfejs v3** (callbacks, NPM, router); nauka „jak robią to inni” jest w ekosystemie v3. |
| [alloy](https://github.com/alloy-rs/alloy) + [docs.rs `alloy`](https://docs.rs/alloy/latest/alloy/) | Standardowy stack **Rust** do `eth_call` / `eth_sendRawTransaction`, kontraktów, typów. |

**Uwaga z sieci / rynku:** niektóre materiały (np. agregatory API typu Compass / QuickNode add-ony) pokazują **skrócone** ścieżki REST — mogą być użyteczne prototypowo, ale **nie** są źródłem prawdy on-chain i często są **płatne**. Dla Waszej filozofii „darmowe dane” traktujcie je jako **opcjonalny** akcelerator, nie dependency produktu.

---

## 2. Ryzyko wielu generacji deployu (must-read przed live)

W `slipstream/README.md` są **trzy** duże tabele: *Initial*, *Gauge Caps*, **Gauges V3**. Tekst upstreamu: *„Existing gauges are still in use, but all new gauges will be deployed from here.”*

**Konsekwencja na live:**

- Pula może być z **innej** `PoolFactory` niż Gauges V3 — **nie zakładajcie**, że `getPool` z jednej fabryki znajdzie każdą parę.
- **Konfiguracja minimalna na produkcję:** albo **bezpośredni adres kontraktu puli** (kanoniczny), albo para tokenów + **tick spacing** + **identyfikator fabryki / generacji** zweryfikowany na BaseScan.
- **Router / NPM / Quoter** muszą być z **tej samej generacji** co pula, albo musicie użyć **MixedQuoterV3** zgodnie z encodingiem wielu fabryk (patrz upstream README przy *MixedRouteQuoterV3*).

W kodzie macie **przypięte adresy Gauges V3** — to jest sensowny default dla **nowych** integracji; dla istniejących pul **weryfikujcie factory z BaseScan**.

---

## 3. Zakres produktu (Definition of Done — etapy)

- **Etap A (komunikacja + read):** REST/OpenAPI, `BASE_RPC_URL`, spójne błędy; odczyty typu `slot0` (i kolejno lekkie `eth_call`) — **bez** obowiązku od razu robić quotera ani tx.
- **Etap B (cięższe on-chain):** symulacja swapu (quoter), budowa i broadcast: `mint` / `increase` / `decrease` / `collect` przez **NPM**, swap przez **SwapRouter** — dopiero po stabilnym A.
- **Poza pierwszym produktem LP:** `deposit` NFT do gauge, `getReward`, logika epok veAERO.

**Fee:** czytajcie fee **z kontraktu / modułów** (Slipstream: dynamiczne fee + ścieżka unstaked fee w spec) — nie kopiujcie „sztywnego tieru” z dokumentacji marketingowej.

---

## 4. Wymagania środowiskowe (przed kodem live)

1. **Base RPC** — stabilny endpoint (własny lub provider), limity QPS, timeouty, ewentualnie drugi fallback (jak na Solanie).
2. **Portfel EVM** — klucz prywatny lub sprzętowy podpis (HSM) — **osobny** od Solany; polityka backupu i rotacji.
3. **ETH na Base** — gas (nie mylić z „ETH na L1”); monitor salda.
4. **ERC-20 approvals** — minimalne kwoty / permit jeśli wspierane; osobna polityka dla routera vs NPM.
5. **Rejestr pul** — allowlista adresów pul / par w konfiguracji (redukcja ryzyka „złej” puli).

---

## 5. Fazy wdrożenia (od najmniejszego ryzyka)

Kolejność jest zgodna z **§0**: najpierw komunikacja i cienki read, dopiero potem symulacje i tx.

### Faza 0 — Komunikacja + lock specyfikacji (1–2 dni)

- **API:** ustalone ścieżki, OpenAPI, zachowanie przy braku RPC / błędzie węzła (już częściowo przez `slot0` + `BASE_RPC_URL`).
- Wybór **domyślnej generacji** (Gauges V3) + reguła: jak trafić na starszą pulę (BaseScan → factory).
- Lista **ABI** (do późniejszych wywołań): repo Slipstream lub BaseScan verified.
- Zapis **konfiguracji**: `BASE_RPC_URL`, docelowo adresy NPM/Router/Quoter (na etap B).

### Faza 1 — Read-only w produkcyjnym RPC (bez klucza podpisującego)

Rozszerzenie tego, co już idzie przez API (`eth_call` w serwisie):

- Kolejne lekkie odczyty: `liquidity`, `token0`/`token1`, ewentualnie tick spacing / fee — **jako osobne endpointy lub agregat**, żeby konsumenci mieli stabilny kontrakt.
- NPM: `positions(tokenId)`, `ownerOf` (osobny podział ścieżek w API).
- Test integracyjny z **jednym** znanym adresem puli z BaseScan.

**Kryterium wyjścia:** powtarzalne odczyty zgodne z UI Aerodrome dla tej samej puli (cena / tick w tolerancji) **przez to samo API**, którego użyje bot/UI.

### Faza 2 — Symulacja zapisu (bez broadcast)

- `eth_call` z bundlem lub kolejnośćą: symulacja `exactInputSingle` / operacji NPM (zależnie od metody — część wymaga `state_override` lub `eth_simulateV1` jeśli dostępne na RPC).
- Porównanie z **Quoter** / **MixedQuoterV3** dla tej samej ścieżki.

**Kryterium wyjścia:** zgodność quote ↔ symulacja w granicach znanych różnic RPC.

### Faza 3 — Anvil / testnet (opcjonalnie)

Base nie zawsze ma bogaty fork lokalny w każdym środowisku — jeśli fork jest dostępny, można replayować tx; jeśli nie, **pomińcie** i idźcie do Fazy 4 z mikrokwotą.

### Faza 4 — Limited live na mainnecie

- **Sufit kapitału** i **sufit dziennej straty** (jak „limited-live” na Solanie).
- Pierwsze tx: `collect` lub minimalny `increase` na małej pozycji, potem swap testowy.
- Logowanie: hash, block, input args, odczyt receipt status.

**Kryterium wyjścia:** N dni bez niezamierzonych revertów i bez driftu konfiguracji adresów.

### Faza 5 — Hardening operacyjny

- Retry nonce, gas bump, obsługa „replacement transaction underpriced”.
- Alerty: brak ETH na gas, revert rate, opóźnienie RPC.
- Runbook: co zrobić przy fork/rug pullu puli (pause bota).

---

## 6. Bezpieczeństwo (minimalny zestaw)

- **Segregacja kluczy:** bot ≠ treasury; hot wallet tylko na operacyjny float.
- **Allowlista pul** i **maksymalne approve** (lub zwiększanie approve per tx tylko o potrzebną kwotę).
- **Dwójka oczu** na pierwszy deploy konfiguracji mainnet (druga osoba porównuje adresy z BaseScan vs repo).
- **Brak auto-approve** „na max” w kodzie produkcyjnym.

---

## 7. Powiązanie z tym monorepo (Solana dziś, Base jutro)

- **Dziś:** Solana Orca itd. — osobny profil operacji.
- **Base Slipstream:** trzymać **EVM** w warstwie `protocols` (lub nowy crate `clmm-lp-evm`), unikać mieszania typów Solana/EVM w `domain` bez jasnego boundary.
- **API/UI:** dodać osobny „connector profile” dopiero gdy read path jest stabilny (żeby nie psuć istniejących endpointów Solany).

---

## 8. Checklist „Go live” (skrót)

- [ ] RPC produkcyjny + fallback + limity.
- [ ] Adresy kontraktów zweryfikowane z BaseScan i zgodne z generacją puli.
- [ ] ABI zgodne z verified source.
- [ ] Read-only test na wybranej puli.
- [ ] Quoter vs symulacja — zaakceptowane odchylenia.
- [ ] Portfel + limity kapitału + approve policy.
- [ ] Runbook operacyjny + alerty.
- [ ] Decyzja: subgraph / indexer pod backtest (osobna ścieżka od live tx).

---

## 9. Linki zewnętrzne (szybkie)

- Slipstream README (deployments): `https://github.com/aerodrome-finance/slipstream/blob/main/README.md`
- Slipstream SPEC: `https://github.com/aerodrome-finance/slipstream/blob/main/SPECIFICATION.md`
- BaseScan (np. podgląd puli): `https://basescan.org/`

---

*Ostatnia aktualizacja planu: 2026-04-11. Przy zmianie upstream deployów — zaktualizujcie `aerodrome_slipstream::gauges_v3` i ten dokument.*
