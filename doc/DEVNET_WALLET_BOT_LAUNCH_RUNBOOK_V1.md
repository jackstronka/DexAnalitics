# Devnet wallet + bot launch runbook v1

Cel: bezpiecznie uruchomic `orca-bot-run` na devnecie i miec powtarzalna procedure, ktora da sie zautomatyzowac w kolejnych iteracjach.

Zakres:
- Orca-first,
- najpierw `dry-run`, potem `limited-live`,
- minimalny kapital testowy,
- operator pod nadzorem (bez "set and forget").

## 0) Model bezpieczenstwa (v1)

Na tym etapie zalecany jest model "hybrydowy light":
- decyzje strategii i petla bota lokalnie,
- podpis lokalnym keypair tylko dla dedykowanego portfela devnet,
- twarde limity operacyjne i checklista GO/NO-GO.

Zasady:
- nigdy nie uzywaj glownego portfela,
- wallet bota trzymaj z malym saldem testowym,
- preferuj `KEYPAIR_PATH`/`SOLANA_KEYPAIR_PATH` zamiast `SOLANA_KEYPAIR` w env,
- nie commituj plikow keypair ani sekretow.

## 1) Prerequisites

- Repo: `F:\CLMM-Liquidity-Provider\CLMM-Liquidity-Provider`
- Rust + cargo dziala lokalnie
- Dostepny RPC devnet: `https://api.devnet.solana.com`
- Masz adres pozycji Whirlpool (`--position`) do monitoringu albo plik JSON z `/tx/open/build` zawierający `position_address`
- Masz dedykowany funded wallet devnet

## 2) Przygotowanie walleta devnet

1. Utworz nowy wallet tylko do bota (dedykowany keypair file).
2. Zasil go minimalna kwota SOL na fee i testowe operacje.
3. Zapisz keypair poza repo (np. katalog domowy operatora).
4. Ustaw lokalnie:

```powershell
$env:SOLANA_RPC_URL="https://api.devnet.solana.com"
$env:KEYPAIR_PATH="C:\secure\devnet-bot\wallet.json"
```

Opcjonalnie fallback RPC:

```powershell
$env:SOLANA_RPC_FALLBACK_URLS="https://api.devnet.solana.com"
```

## 3) Gate 1: twardy preflight

Przed kazdym runem sprawdz:
- [ ] `SOLANA_RPC_URL` ustawione,
- [ ] `KEYPAIR_PATH` istnieje i wskazuje poprawny plik,
- [ ] `cargo` dziala,
- [ ] bot startuje bez bledow konfiguracji.

Szybki preflight:

```powershell
if (-not $env:SOLANA_RPC_URL) { throw "Missing SOLANA_RPC_URL" }
if (-not $env:KEYPAIR_PATH) { throw "Missing KEYPAIR_PATH" }
if (-not (Test-Path $env:KEYPAIR_PATH)) { throw "Missing keypair file: $env:KEYPAIR_PATH" }
cargo --version
```

## 4) Gate 2: devnet E2E tests (obowiazkowe)

Odpal przed pierwszym live i po istotnych zmianach tx/lifecycle:

```powershell
cargo test -p clmm-lp-api devnet_ -- --ignored
```

Minimalne kryterium przejscia:
- [ ] testy devnet lifecycle / unsigned flow przechodza stabilnie,
- [ ] brak krytycznych bledow policy/simulate/send.

## 5) Start sesji: dry-run (default)

Komenda:

```powershell
cargo run --bin clmm-lp-cli -- orca-bot-run `
  --position <POSITION_PUBKEY> `
  --eval-interval-secs 300 `
  --poll-interval-secs 30
```

Co obserwowac:
- [ ] petla monitor + strategy startuje poprawnie,
- [ ] decyzje `Hold` i `Rebalance` pojawiaja sie zgodnie z rynkiem,
- [ ] brak petli tych samych bledow.

Kryterium zaliczenia dry-run:
- [ ] co najmniej jedna decyzja `Hold`,
- [ ] co najmniej jedna decyzja `Rebalance` (w logice bota),
- [ ] brak krytycznych nieobsluzonych bledow.

## 6) Start sesji: limited-live (`--execute`)

Uruchamiaj tylko po zaliczonym dry-run:

```powershell
cargo run --bin clmm-lp-cli -- orca-bot-run `
  --position <POSITION_PUBKEY> `
  --execute `
  --eval-interval-secs 300 `
  --poll-interval-secs 30
```

Ograniczenia operacyjne (must):
- [ ] jeden pool/pozycja na start,
- [ ] maly kapital testowy,
- [ ] operator online podczas calej sesji,
- [ ] gotowa procedura stop/rollback.

## 7) GO/NO-GO (sesja)

GO, jesli:
- [ ] dry-run byl stabilny,
- [ ] brak krytycznych alertow,
- [ ] tx flow (simulate -> send -> confirm) jest stabilny,
- [ ] operator ma aktywny kanal monitoringu i plan awaryjny.

NO-GO, jesli:
- [ ] powtarzalne tx failures bez root cause,
- [ ] niespojnosc decyzja vs wykonanie,
- [ ] problemy wallet/config integrity.

## 8) Post-run audit (obowiazkowe)

Po sesji zapisz:
- data/godzina i tryb (`dry-run`/`limited-live`),
- uzyte ENV i parametry runu,
- liczba decyzji hold/rebalance,
- lista tx signatures (jesli execute),
- incidenty + co poprawic przed kolejnym runem.

To jest podstawa do automatyzacji i do przyszlego "stage 1 mainnet readiness".

## 9) Automatyzacja (PowerShell v1)

W repo sa gotowe 3 skrypty:

1. `tools/bot_preflight.ps1`
   - waliduje ENV, keypair path, opcjonalnie sanity RPC,
   - zwraca niezerowy exit code na brakach.

2. `tools/bot_run_devnet.ps1`
   - parametry: `-Position` **albo** `-OpenBuildResponseJson`, `-Execute`, `-EvalIntervalSecs`, `-PollIntervalSecs`,
   - odpala `orca-bot-run` ze standaryzowanymi flagami,
   - domyslnie odpala preflight (mozna pominac przez `-SkipPreflight`).

3. `tools/bot_postrun_report.ps1`
   - zbiera metadane runu i status,
   - zapisuje JSON raportu do `data/reports/`.

4. `tools/bot_session_devnet.ps1`
   - laczy caly flow: preflight -> run -> post-run report,
   - przyjmuje `-Position` lub `-OpenBuildResponseJson` i zawsze bierze realny `position_address` z odpowiedzi API open,
   - nawet gdy run failnie lub dojdzie timeout, zapisuje raport (`run_status=failed` / `run_status=timeout`),
   - opcja `-MaxRuntimeMinutes` zatrzymuje sesje automatycznie po czasie.

### Przyklady

Preflight:

```powershell
.\tools\bot_preflight.ps1
```

Dry-run (z preflight, z pozycji podanej bezposrednio):

```powershell
.\tools\bot_run_devnet.ps1 -Position <POSITION_PUBKEY>
```

Dry-run (z `position_address` zwroconego przez `/tx/open/build`):

```powershell
.\tools\bot_run_devnet.ps1 -OpenBuildResponseJson .\tmp\open_build_response.json
```

Limited-live (z preflight + execute):

```powershell
.\tools\bot_run_devnet.ps1 -Position <POSITION_PUBKEY> -Execute
```

Post-run report:

```powershell
.\tools\bot_postrun_report.ps1 `
  -Position <POSITION_PUBKEY> `
  -Mode limited-live `
  -EvalIntervalSecs 300 `
  -PollIntervalSecs 30 `
  -Signatures "sig1,sig2" `
  -Notes "First supervised limited-live session"
```

One-command session:

```powershell
.\tools\bot_session_devnet.ps1 `
  -Position <POSITION_PUBKEY> `
  -Execute `
  -MaxRuntimeMinutes 45 `
  -EvalIntervalSecs 300 `
  -PollIntervalSecs 30 `
  -Notes "Supervised session"
```

## 10) Kierunek v2 (bezpieczniejszy podpis)

Po ustabilizowaniu v1:
- przejscie do modelu "unsigned tx + external signer/policy gate",
- limity per wallet i allowlist programow na warstwie podpisu,
- rotacja kluczy i separacja roli decision-engine vs signer.

To zmniejsza ryzyko ekspozycji private key w procesie bota.
