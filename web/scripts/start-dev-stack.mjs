/**
 * Dev stack jak w typowych monorepo: concurrently + krótki „splash”, szybkie zwolnienie portów.
 * Ctrl+C kończy API i Vite (KillOnSignal / killOthersOn).
 *
 * Opcje env:
 *   CLMM_OPEN_BROWSER=true — `vite --open`
 *   RUST_LOG — domyślnie `info`
 */
import concurrently from 'concurrently'
import { execSync } from 'child_process'
import { createRequire } from 'module'
import path from 'path'
import { fileURLToPath } from 'url'
import p from 'picocolors'

const require = createRequire(import.meta.url)
const killPort = require('kill-port')

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const webDir = path.resolve(__dirname, '..')
const repoRoot = path.resolve(webDir, '..')

/** Równolegle: taskkill/pkill api + zwolnienie 3000 oraz portu API (domyślnie 8081; krótki sleep na Windows pod plik .exe). */
async function freeDevPorts() {
  const apiPort = Number.parseInt(process.env.API_PORT || '8081', 10)
  const jobs = []
  if (process.platform === 'win32') {
    jobs.push(
      Promise.resolve().then(() => {
        try {
          execSync('taskkill /F /IM clmm-lp-api.exe /T', { stdio: 'ignore' })
        } catch {
          /* brak procesu */
        }
      }),
    )
  } else {
    jobs.push(
      Promise.resolve().then(() => {
        try {
          execSync('pkill -f clmm-lp-api 2>/dev/null || true', { stdio: 'ignore', shell: true })
        } catch {
          /* ok */
        }
      }),
    )
  }
  jobs.push(killPort(3000).catch(() => {}))
  jobs.push(killPort(apiPort).catch(() => {}))
  await Promise.all(jobs)
  await new Promise((r) => setTimeout(r, 60))
}

await freeDevPorts()

const openBrowser =
  process.env.CLMM_OPEN_BROWSER === '1' || process.env.CLMM_OPEN_BROWSER === 'true'
// Use `npx` so it works even when PATH lacks node_modules/.bin (e.g. direct node invocation).
const webCmd = openBrowser ? 'npx vite --open' : 'npx vite'
const apiPort = Number.parseInt(process.env.API_PORT || '8081', 10)

console.log()
console.log(
  p.cyan('▶ ') +
    p.bold('Bociarz LP') +
    p.dim(`  http://localhost:3000  ·  API :${apiPort}  ·  Ctrl+C → stop`) +
    (openBrowser ? p.dim('  ·  --open') : ''),
)
console.log()

const env = {
  ...process.env,
  CLMM_REPO_ROOT: repoRoot,
  RUST_LOG: process.env.RUST_LOG || 'info',
  API_PORT: String(apiPort),
  // Keep Vite proxy in sync with the API port (helps when :8080 is occupied locally).
  API_UPSTREAM: process.env.API_UPSTREAM || `http://127.0.0.1:${apiPort}`,
}

const { result } = concurrently(
  [
    { command: 'cargo run --bin clmm-lp-api', name: 'api', cwd: repoRoot, env },
    { command: webCmd, name: 'web', cwd: webDir, env },
  ],
  {
    prefix: 'name',
    padPrefix: true,
    timings: true,
    prefixColors: ['#0ea5e9', '#c026d3'],
    killOthersOn: ['failure', 'success'],
    restartTries: 0,
  },
)

try {
  await result
} catch {
  process.exit(1)
}
