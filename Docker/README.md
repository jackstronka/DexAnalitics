# Docker Deployment

This directory contains Docker configuration for deploying the **Bociarz LP Strategy Lab** stack (derived from CLMM Liquidity Provider).

## Contents

| File | Description |
|------|-------------|
| `docker-compose.yml` | Docker Compose/Swarm configuration |
| `api.Dockerfile` | API server image |
| `cli.Dockerfile` | CLI tool image |
| `web.Dockerfile` | Web dashboard image |
| `nginx.conf` | Nginx configuration for web dashboard |
| `.env.example` | Environment variables template |

## Quick Start

### Local Development

```bash
# Copy environment file
cp .env.example .env
# Edit .env with your values

# Build and start all services
docker-compose up -d

# View logs
docker-compose logs -f

# Stop services
docker-compose down
```

### Docker Swarm Deployment

```bash
# Initialize swarm (if not already done)
docker swarm init

# Copy and configure environment
cp .env.example .env
# Edit .env with production values

# Deploy stack
docker stack deploy -c docker-compose.yml clmm-lp

# View services
docker service ls

# View logs
docker service logs clmm-lp_api -f

# Scale services
docker service scale clmm-lp_api=3 clmm-lp_web=3

# Remove stack
docker stack rm clmm-lp
```

## Building Images Locally

```bash
# From the repository root
cd ..

# Build API image
docker build -f Docker/api.Dockerfile -t clmm-lp-api .

# Build CLI image
docker build -f Docker/cli.Dockerfile -t clmm-lp-cli .

# Build Web image
docker build -f Docker/web.Dockerfile -t clmm-lp-web .
```

## Using Pre-built Images

Images are automatically built and pushed to GitHub Container Registry on every release:

```bash
# Pull images
docker pull ghcr.io/joaquinbejar/clmm-liquidity-provider/api:latest
docker pull ghcr.io/joaquinbejar/clmm-liquidity-provider/cli:latest
docker pull ghcr.io/joaquinbejar/clmm-liquidity-provider/web:latest
```

## Running CLI Commands

```bash
# Initialize database
docker run --rm \
  -e DATABASE_URL=postgres://user:pass@host:5432/db \
  ghcr.io/joaquinbejar/clmm-liquidity-provider/cli:latest \
  db init

# Run analysis
docker run --rm \
  -e BIRDEYE_API_KEY=your_key \
  ghcr.io/joaquinbejar/clmm-liquidity-provider/cli:latest \
  analyze --symbol-a SOL --symbol-b USDC --days 30
```

## Service Ports

| Service | Internal Port | External Port |
|---------|---------------|---------------|
| PostgreSQL | 5432 | 5432 |
| API Server | 8080 | 8080 |
| Web Dashboard | 80 | 3000 |

## Health Checks

All services include health checks:

- **PostgreSQL**: `pg_isready`
- **API**: `GET /api/v1/health`
- **Web**: `GET /health`

## Resource Limits

Default resource limits (configurable in docker-compose.yml):

| Service | CPU Limit | Memory Limit |
|---------|-----------|--------------|
| PostgreSQL | 1 | 1GB |
| API | 2 | 2GB |
| Web | 0.5 | 256MB |

## Volumes

- `postgres_data`: Persistent PostgreSQL data

## Networks

- `clmm-network`: Overlay network for service communication

## Security Notes

1. **Change default passwords** in production
2. **Use secrets management** for sensitive values
3. **Enable TLS** for external access
4. **Restrict network access** to necessary ports only
