# TODO: warstwa screenshot + agent (publiczne UI DEX)

**Status:** plan do realizacji (nie wdrożone w kodzie).  
**Cel:** opcjonalny **drugi głos** przy decyzjach strategii — analiza **wykresów z publicznych stron** Orca / Raydium / Meteora, bez wysyłania danych portfela.

**Powiązane:** [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) (prawda on-chain z RPC, `AgentDecision`, `POST .../apply-optimize-result`, `optimize_apply_policy`), [`BOT_OPERATIONS_MODEL_2026-03-23.md`](BOT_OPERATIONS_MODEL_2026-03-23.md) (tryby, eskalacja — snapshot). Typy: [`crates/domain/src/agent_decision.rs`](../crates/domain/src/agent_decision.rs).

---

## Osobny profil / tryb (nie domyślna ścieżka)

Cała warstwa (screenshot, wielu agentów, konsensus, rulebooki) ma być **świadomie włączana** jako **odrębny profil operacyjny** — **bez mieszania** z klasycznym potokiem „tylko optimize + `AgentDecision` / tylko subprocess”.

- **Domyślnie wyłączone:** brak screenshotów, brak blackboardu, brak dodatkowych kosztów LLM — zachowanie jak dziś (RPC + strategia + ewentualnie istniejący apply optimize).
- **Włączenie jawne:** jeden przełącznik w konfiguracji strategii / operatora (nazwa robocza: np. `agent_layer_profile` = `none` | `chart_assist` | `multi_agent_consensus` — do ustalenia przy implementacji). Zmiana profilu = **widoczny** w logach i w audycie.
- **Izolacja:** ten profil **nie nadpisuje** `optimize_apply_policy` ani nie „przechodzi na autopilot” bez reguł — to osobna warstwa **nad** lub **obok** istniejących polityk, z własnymi limitami (timeout, max rund, koszt).
- **Cel:** żeby zespół mógł testować konsensus i reguły na **wybranych** strategiach, podczas gdy reszta środowiska zostaje na prostym modelu decyzyjnym.

---

## Połączenie z innymi agentami (orchestracja)

Agent od **wykresu** ma być **częścią większego układu agentów**, nie izolowaną usługą.

- **Rola „chart agent”:** tylko **wejście wizualne** → ustrukturyzowany sygnał (np. `ChartOpinion` / JSON z `confidence`, `abstain`, `regime`). Bez samodzielnego składania `OptimizeResultFile` ani bypassu API strategii, chyba że jawnie zaprojektujecie taki krok z audytem.
- **Kompozycja z istniejącym agentem optymalizacji:** envelope HTTP (`AgentDecision` + ewentualnie `baseline_optimize_result`) może w przyszłości zawierać **opcjonalne pole** z ostatnią opinią wykresu (wersja polityki + timestamp + hash obrazka), albo **orkiestrator** zbiera opinie z wielu agentów zanim złoży finalne `AgentDecision`.
- **Zasada łączenia:** jasno opisane **reguły agregacji** (np. chart agent może tylko obniżyć `approved` / wymusić `abstain`, gdy `confidence` niskie lub `chart_quality != ok`; agent od ryzyka ma pierwszeństwo przy konflikcie X — do zapisania w dokumencie polityki).
- **Ślad audytowy:** każda decyzja wieloagentowa loguje **który agent** co dostarczył (`schema_version` per agent, `reason`).

---

## Tryb konsensusu (osobny wariant wieloagentowy)

Oprócz prostych reguł agregacji („chart może tylko veto”) możliwy jest **osobny tryb pracy**: agenci **wymieniają się informacjami** i **dążą do jednej wspólnej decyzji**, zamiast tylko dopinać sygnały szeregowo.

**Idea:**

- **Wspólna przestrzeń kontekstu** („blackboard” / transcript): każdy agent dokłada **ustrukturyzowany** wpis (np. hipoteza, `confidence`, brakujące dane, zastrzeżenia). Kolejna runda widzi **sumaryczny stan** dyskusji, nie tylko własny poprzedni output.
- **Jedno wyjście na zewnątrz:** po zakończeniu trybu konsensusu nadal wychodzi **jeden** dokument zgodny z produktem (np. pojedynczy `AgentDecision` + załącznik z pełnym transcriptem do audytu).
- **Zakończenie pętli:** albo **zgoda** (wszyscy powyżej progu / brak veto), albo **limit rund / timeout**, albo **jawny impas**.

**Ryzyka i świadome kompromisy:**

- **Koszt i czas:** wiele wywołań modeli na rundę; tryb konsensusu powinien być **rzadki** lub **zdarzeniowy**, nie domyślny na każdy tick.
- **Powtarzalność:** transcript + niskie `temperature` + sztywne schema per runda; i tak możliwy **rozrzut** między uruchomieniami — stąd **twarde fallbacki** poniżej.
- **Brak zgody:** polityka musi definiować **co wtedy**: np. `abstain` / `approved: false` / „nie aplikuj optimize” / eskalacja do człowieka — **nigdy** nieokreślone zachowanie przy impasie.

**Odróżnienie od hierarchii:** w trybie hierarchicznym jeden „szef” łączy opinie bez dialogu. W **konsensusie** agenci **reagują na treść** wpisów innych (nawet jeśli technicznie realizuje to jeden model w wielu krokach z transcriptem — to kwestia implementacji).

---

## Model operacyjny (ustalone założenia)

- **Brak logowania** w przeglądarce — wejście wyłącznie po **publiczny** widok (wykres / pool), zrobienie screenshotu, wyjście. Nic poza tym w tej sesji.
- **Źródło:** tylko publiczne strony **Orca, Raydium, Meteora** (nie panel z saldami / podłączonym portfelem).
- **Kadrowanie:** stały szablon — preferowany **tylko obszar wykresu** (+ ewentualnie para/pool w legendzie), bez paska adresów, zakładek, całego desktopu.
- **Częstotliwość:** nie musi być wysoka — możliwe **rzadkie** uruchomienia oraz **wyzwalanie zdarzeniem** (trigger z własnych metryk / alertu), żeby ograniczyć koszt API.
- **Koszty:** dopuszczalne modele darmowe / płatne — warstwa **wymienna** (jeden kontrakt typu „dostawca opinii z obrazu”).

---

## Zasady bezpieczeństwa decyzji

- Agent **nigdy** jako jedyne źródło instrukcji on-chain; **twarde reguły** (RPC, strategia, limity) pozostają źródłem prawdy.
- Dozwolone działania po stronie agenta (do ustalenia w implementacji): np. `bias_score`, `abstain`, `suggest_review`, ewentualnie **veto / opóźnienie** z progiem i logiem audytu.
- **Powtarzalność:** wersjonowany plik zasad (co brać pod uwagę, co ignorować, czego nie uznawać za pewnik) + **ustrukturyzowane wyjście** (np. JSON ze schematem, niskie `temperature` dla części tekstowej).
- **Dedup:** hash / perceptual hash obrazka — pomijać ponowną analizę przy identycznym lub prawie identycznym wykresie.
- **Retencja:** automatyczne usuwanie / limit liczby plików screenshotów po analizie (żeby nie rosło bez końca).

---

## „Trening” agentów przez reguły (bez fine-tuningu na start)

W praktyce **główny nośnik zachowania** nie musi być klasycznym treningiem modelu (SFT/RLHF), tylko **jawnie zapisanym pakietem reguł** ładowanym przy każdym wywołaniu:

- **Reguły biznesowe i ostrożności** — co wolno wnioskować z wykresu, co jest zabronione, kiedy obowiązkowo `abstain`, jak mapować wzorce na etykiety (`regime`, ryzyko).
- **Rubryka / checklista** — krok po kroku „najpierw ocen jakość obrazu, potem trend, potem niepewność”, żeby ograniczyć fantazję modelu.
- **Przykłady referencyjne (few-shot)** — krótkie, wersjonowane: „dla takiego opisu wykresu → taki JSON wyjściowy”; ułatwia powtarzalność między modelami.
- **Osobny zestaw reguł per rola** — chart vs optymalizacja vs ryzyko vs moderator konsensusu; wspólny **numer wersji pakietu** w logach (`ruleset_version`).

**Ewaluacja:** zbiór **sztucznych przypadków** (obrazy + oczekiwane pola JSON lub oczekiwany wynik konsensusu); uruchamiany **offline** przy zmianie reguł — regresja zanim reguły trafią na produkcję.

**Opcjonalnie później:** prawdziwy **fine-tuning** lub adaptacja modelu pod Wasz format — tylko jeśli reguły + schema + ewaluacja przestaną wystarczać; wtedy osobny dokument i koszt.

---

## Backlog (do odhaczania)

- [ ] **P14** (Content) “Bociarz LP Strategy Lab” — pipeline narracji do filmów: generuj segmenty (`studio-stream-plan`) → TTS “moim głosem” (voice clone) → montaż/wrzutka na YouTube. Założenie budżetu startowego: **~100 min/mies** głosu wystarczy do regularnych filmów (np. ~10×10 min albo ~20×5 min narracji).
- [ ] **P15** (Cost) Ustalić plan TTS dla voice clone: na start sensowny jest próg ~100 min/mies (np. ElevenLabs Creator, rzędu **$22/mies** wg cennika), a live 24/7 liczyć osobno po minutach.

- [ ] **P1** Opisać w repo **wersjonowany** szablon polityki agenta (`version`, **reguły jako główny „trening”**, rubryki, few-shot, format wyjścia).
- [ ] **P2** Zdefiniować **JSON schema** odpowiedzi (np. `regime`, `confidence`, `abstain`, `chart_quality`) i walidację po stronie aplikacji.
- [ ] **P3** Lista **triggerów** (czas rzadki vs zdarzenia z monitora / snapshotów) + dokumentacja „kiedy nie wołać modelu”.
- [ ] **P4** Katalog screenshotów (np. `data/chart-captures/`) + polityka **TTL / max plików**.
- [ ] **P5** Integracja z botem: tylko **sygnał pomocniczy** podłączony do istniejącego potoku decyzyjnego (Orca live).
- [ ] **P6** Sprawdzenie **ToS** stron DEX oraz dostawcy modelu dla screenshotów stron trzecich.
- [ ] **P7** **Orchestracja wieloagentowa:** dokument „kto z kim” (chart agent ↔ agent `AgentDecision` / optymalizacja ↔ ewentualni inni); reguły agregacji i priorytety przy sprzeczności.
- [ ] **P8** Kontrakt danych: rozszerzenie envelope / osobny endpoint / kolejka wewnętrzna — spójnie z `apply-optimize-result` i `optimization_busy` (bez wyścigów między agentami).
- [ ] **P9** Specyfikacja **trybu konsensusu:** format blackboardu / transcriptu, max rund, timeout, progi zgody, identyfikacja ról agentów (chart / optimize / risk / …).
- [ ] **P10** **Fallback przy braku konsensusu** + testy regresyjne zachowania (zawsze jednoznaczny wynik dla warstwy wykonawczej).
- [ ] **P11** **Rulebook per agent** w repo (`ruleset_version`, zmiana = PR + ewaluacja); wspólne zasady nazewnictwa i spójność z trybem konsensusu.
- [ ] **P12** **Harness ewaluacyjny** (fixtures: obrazy/transcripty → oczekiwany JSON / oczekiwany wynik agregacji); uruchomienie w CI lub przed wdrożeniem reguł.
- [ ] **P13** **Profil w konfiguracji:** pojedyncze pole / enum (`agent_layer_profile` lub równoważne), dokumentacja wartości, domyślnie `none`; testy że `none` nie woła warstwy agentów.

---

## Log wykonania

| Data | Krok | Notatka |
|------|------|---------|
| 2026-03-26 | Plan | Utworzono plik; założenia: brak logowania, tylko publiczne UI Orca/Raydium/Meteora, crop, triggery, koszt pod kontrolą. |
| 2026-03-26 | Plan | Sekcja orchestracji: chart agent jako część układu z innymi agentami (`AgentDecision` / apply-optimize); P7–P8. |
| 2026-03-26 | Plan | Tryb **konsensusu**: wspólny blackboard/transcript, jedna decyzja wyjściowa, koszty/limit rund/fallback; P9–P10. |
| 2026-03-26 | Plan | Sekcja **reguły jako trening**: rulebook, rubryki, few-shot, ewaluacja offline; P11–P12; fine-tuning opcjonalnie później. |
| 2026-03-26 | Plan | **Osobny profil/tryb** (`agent_layer_profile`), domyślnie wyłączone; P13. |
| 2026-03-26 | Plan | Content: “Bociarz LP Strategy Lab” — voice clone (TTS) do filmów; startowy budżet ~100 min/mies; P14–P15. |
