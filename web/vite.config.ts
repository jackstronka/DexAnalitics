import { defineConfig, loadEnv } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'
import fs from 'fs'

// https://vite.dev/config/
export default defineConfig(({ mode }) => {
  const cwdEnv = loadEnv(mode, process.cwd(), '')
  const repoRoot = path.resolve(__dirname, '..')
  const rootEnv = loadEnv(mode, repoRoot, '')
  /** `web/.env*` overrides repo root — root supplies `API_PORT` when only `npm run dev` runs (bez concurrently). */
  const env = { ...rootEnv, ...cwdEnv }
  /** W Dockerze proxy musi iść do serwisu `api`, nie do localhost w kontenerze `web`. */
  const apiPort = env.API_PORT || '8080'
  const apiUpstream = env.API_UPSTREAM || `http://127.0.0.1:${apiPort}`
  const apiWs = apiUpstream.replace(/^http/, 'ws')
  const bindAll = env.VITE_DOCKER === '1' || env.VITE_BIND_ALL === '1'
  const wsProxyLogEnabled = (env.VITE_WS_PROXY_LOG ?? '1') !== '0'
  const wsProxyLogPath = path.join(repoRoot, 'tools', 'logs', 'vite-ws-proxy.log')

  const appendWsProxyLog = (line: string) => {
    if (!wsProxyLogEnabled) return
    try {
      fs.mkdirSync(path.dirname(wsProxyLogPath), { recursive: true })
      fs.appendFileSync(wsProxyLogPath, `${new Date().toISOString()} ${line}\n`, 'utf8')
    } catch {
      // best-effort; never crash dev server because of logging
    }
  }

  return {
    plugins: [react()],
    resolve: {
      alias: {
        '@': path.resolve(__dirname, './src'),
      },
    },
    server: {
      port: 3000,
      ...(bindAll ? { host: true as const } : {}),
      watch: {
        usePolling: env.CHOKIDAR_USEPOLLING === 'true',
      },
      proxy: {
        '/api': {
          target: apiUpstream,
          changeOrigin: true,
        },
        '/ws': {
          target: apiWs,
          ws: true,
          // `node-http-proxy` has request/socket timeouts that can surface as dev WS resets
          // (commonly around ~120s) when proxying long-lived connections. Disable them for dev.
          timeout: 0,
          proxyTimeout: 0,
          // API routes are versioned under `/api/v1`, while the frontend connects to `/ws/*`
          // on the Vite dev server. Rewrite to keep local dev URLs clean.
          rewrite: (p) => p.replace(/^\/ws\b/, '/api/v1/ws'),
          configure: (proxy, _options) => {
            if (!wsProxyLogEnabled) return

            // Best-effort hardening: ensure runtime options are applied even if underlying
            // types differ between http-proxy versions.
            try {
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
              const p: any = proxy as any
              if (p?.options) {
                p.options.timeout = 0
                p.options.proxyTimeout = 0
              }
            } catch {
              // ignore
            }

            proxy.on('error', (err, req) => {
              // `req` may be an http.IncomingMessage (for WS upgrade too)
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
              const url = (req as any)?.url ?? '(unknown-url)'
              appendWsProxyLog(`proxy_error url=${url} error=${err?.code ?? ''} ${err?.message ?? err}`)
            })
            // WS-specific hooks provided by http-proxy
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            proxy.on('proxyReqWs', (_proxyReq: any, req: any) => {
              const url = req?.url ?? '(unknown-url)'
              appendWsProxyLog(`proxy_req_ws url=${url} target=${apiWs}`)
            })
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            proxy.on('open', (_proxySocket: any) => {
              appendWsProxyLog('proxy_ws_open')
            })
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            proxy.on('close', (_res: any, _socket: any, _head: any) => {
              appendWsProxyLog('proxy_ws_close')
            })
          },
        },
      },
    },
  }
})
