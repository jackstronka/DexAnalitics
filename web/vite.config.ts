import { defineConfig, loadEnv } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'

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
        },
      },
    },
  }
})
