# Docker (frontend + API) — jak to się robi „w świecie”

## Wymagania (bez tego `docker compose` się nie połączy)

### Windows

1. Zainstaluj **[Docker Desktop](https://www.docker.com/products/docker-desktop/)** (silnik **Linux containers**).
2. **Uruchom Docker Desktop** i poczekaj, aż status będzie **Running** (ikona w zasobniku).
3. Dopiero wtedy w katalogu repo: `docker compose up --build`

**Typowy błąd**, gdy Desktop jest zatrzymany lub nie zainstalowany:

```text
open //./pipe/dockerDesktopLinuxEngine: Nie można odnaleźć określonego pliku.
```

To oznacza: klient Docker nie może połączyć się z silnikiem — **włącz Docker Desktop** albo napraw instalację (WSL2 / Hyper-V zgodnie z dokumentacją Dockera).

### macOS / Linux

Docker Engine + Compose plugin (lub Docker Desktop na Macu) — ten sam warunek: **daemon musi działać** (`docker info` bez błędu).

---

## Idea

- **Compose** — kilka usług w jednej sieci DNS (`api`, `web`), stałe porty na hoście (`8080`, `3000`).
- **Dev**: obraz z Node/Rust + **bind mount** kodu z hosta → HMR / szybka iteracja (jak w tutorialach Vite + backend).
- **Prod** (typowo): osobno — `npm run build` → statyczne pliki na **nginx** / CDN / S3; API jako osobny deployment; to nie jest ten sam `docker-compose` co lokalny dev.

## W tym repo

Z katalogu głównego repozytorium:

```bash
docker compose up --build
```

- **Dashboard**: http://localhost:3000  
- **API**: http://localhost:8080 (Swagger: `/docs`)

**Ważne:** dev serwer Vite w kontenerze **nie może** proxyować na `localhost:8080` — tam jest tylko sam kontener `web`. Dlatego w Compose jest `API_UPSTREAM=http://api:8080`; `web/vite.config.ts` czyta **`API_UPSTREAM`** (tylko po stronie Node dev serwera, nie trafia do bundle przeglądarki).

## Zmienne (skrót)

| Zmienna | Gdzie | Znaczenie |
|--------|--------|-----------|
| `API_UPSTREAM` | `web` (Compose) | URL backendu dla proxy `/api` i `/ws` |
| `VITE_DOCKER=1` | `web` | `server.host` → nasłuch `0.0.0.0` (dostęp z hosta przez mapowanie portów) |
| `CHOKIDAR_USEPOLLING=true` | `web` | często potrzebne przy volume na Windows/macOS, żeby HMR widział zmiany plików |
| `CLMM_REPO_ROOT=/repo` | `api` | montowane jest całe repo w `/repo` (Scripts / manifesty) |

Lokalnie bez Dockera nic nie zmieniasz — domyślnie proxy idzie na `http://127.0.0.1:8080`.

## Pierwszy build API

Obraz **`docker/api`** robi `cargo build --release -p clmm-lp-api` — przy dużym drzewie zależności (Solana, Orca) **pierwszy raz może trwać długo**; to normalne. Kolejne buildy korzystają z cache warstw Dockera, o ile nie zmienisz `Cargo.toml` / crate’ów.

## Typowe problemy

- **Port zajęty** — zatrzymaj lokalne `clmm-lp-api` / Vite albo zmień mapowanie w `docker-compose.yml` (`3001:3000` itd.).
- **Brak miejsca na obrazy** — `docker system prune` (ostrożnie: usuwa nieużywane obrazy).
- **Firewall / Docker Desktop** — porty muszą być dostępne z hosta (domyślnie tak).

## Produkcja (orientacyjnie)

Ten plik jest pod **development**. Na produkcji frontend to zwykle:

1. `npm run build` w CI,
2. wrzucenie `web/dist` za nginx lub do hostingu statycznego,
3. API osobno (K8s, VM, managed container) z właściwym `SOLANA_RPC_URL` i sekretami — **nie** commituj sekretów do Compose.
