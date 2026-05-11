// WebSocket client for real-time updates

export interface PositionUpdate {
  type: 'position_update'
  position_address: string
  timestamp: string
  data: {
    value_usd: string
    pnl_percent: string
    il_percent: string
    in_range: boolean
  }
}

export interface AlertUpdate {
  type: 'alert'
  level: 'info' | 'warning' | 'critical'
  title: string
  message: string
  timestamp: string
}

export type WebSocketMessage = PositionUpdate | AlertUpdate

type MessageHandler = (message: WebSocketMessage) => void

class WebSocketClient {
  private ws: WebSocket | null = null
  private handlers: Set<MessageHandler> = new Set()
  private reconnectAttempts = 0
  private maxReconnectAttempts = 5
  private reconnectDelay = 1000
  private endpoint: string | null = null
  private shouldReconnect = true

  private wsUpstream(): string | null {
    // Optional dev override to bypass Vite WS proxy (/ws -> /api/v1/ws).
    // Accept either http(s)://... or ws(s)://...
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const raw = (((import.meta as any).env?.VITE_WS_UPSTREAM ?? '') as string).trim()
    if (!raw) return null
    if (raw.startsWith('ws://') || raw.startsWith('wss://')) return raw
    if (raw.startsWith('http://')) return raw.replace(/^http:\/\//, 'ws://')
    if (raw.startsWith('https://')) return raw.replace(/^https:\/\//, 'wss://')
    return null
  }

  private toApiWsPath(endpoint: string): string {
    // UI uses clean `/ws/*` paths in dev; API is versioned under `/api/v1/ws/*`.
    return endpoint.replace(/^\/ws\b/, '/api/v1/ws')
  }

  private debugEnabled() {
    // On by default in dev; set `VITE_WS_CLIENT_LOG=0` in `web/.env.local` to disable.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const flag = ((import.meta as any).env?.VITE_WS_CLIENT_LOG ?? '1') as string
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const isDev = Boolean((import.meta as any).env?.DEV)
    return isDev && flag !== '0'
  }

  private debugLog(event: Record<string, unknown>) {
    if (!this.debugEnabled()) return
    try {
      const key = 'ws_debug_log_v1'
      const raw = window.localStorage.getItem(key)
      const items = (raw ? (JSON.parse(raw) as unknown[]) : []).slice(-200)
      items.push({ ts: new Date().toISOString(), endpoint: this.endpoint, ...event })
      window.localStorage.setItem(key, JSON.stringify(items))
    } catch {
      // ignore
    }
  }

  connect(endpoint: string) {
    // Prevent duplicate sockets if called multiple times rapidly (dev StrictMode, HMR, etc).
    if (
      this.ws?.readyState === WebSocket.OPEN ||
      this.ws?.readyState === WebSocket.CONNECTING
    ) {
      return
    }

    this.shouldReconnect = true
    this.endpoint = endpoint
    const upstream = this.wsUpstream()
    const wsUrl = upstream
      ? `${upstream}${this.toApiWsPath(endpoint)}`
      : `${window.location.protocol === 'https:' ? 'wss:' : 'ws:'}//${window.location.host}${endpoint}`

    this.ws = new WebSocket(wsUrl)

    this.ws.onopen = () => {
      console.log('WebSocket connected')
      this.debugLog({ type: 'open', url: wsUrl })
      this.reconnectAttempts = 0
    }

    this.ws.onmessage = (event) => {
      try {
        const message = JSON.parse(event.data) as WebSocketMessage
        this.handlers.forEach(handler => handler(message))
      } catch (error) {
        console.error('Failed to parse WebSocket message:', error)
        this.debugLog({ type: 'message_parse_error', error: String(error) })
      }
    }

    this.ws.onclose = (ev) => {
      console.log('WebSocket disconnected')
      this.debugLog({ type: 'close', code: ev.code, reason: ev.reason, wasClean: ev.wasClean })
      if (this.shouldReconnect) {
        this.attemptReconnect(endpoint)
      } else {
        this.reconnectAttempts = 0
      }
    }

    this.ws.onerror = (error) => {
      console.error('WebSocket error:', error)
      this.debugLog({ type: 'error', error: String(error) })
    }
  }

  private attemptReconnect(endpoint: string) {
    if (this.reconnectAttempts < this.maxReconnectAttempts) {
      this.reconnectAttempts++
      const delay = this.reconnectDelay * Math.pow(2, this.reconnectAttempts - 1)
      console.log(`Reconnecting in ${delay}ms (attempt ${this.reconnectAttempts})`)
      setTimeout(() => this.connect(endpoint), delay)
    }
  }

  disconnect() {
    if (this.ws) {
      // Intentional close (e.g., component unmount). Do not auto-reconnect.
      this.shouldReconnect = false
      this.ws.close()
      this.ws = null
    }
  }

  subscribe(handler: MessageHandler) {
    this.handlers.add(handler)
    return () => this.handlers.delete(handler)
  }

  send(message: unknown) {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(message))
    }
  }
}

export const positionsWs = new WebSocketClient()
export const alertsWs = new WebSocketClient()

export function connectWebSockets() {
  // Dev toggle: disable WS to avoid Vite proxy spam during frequent API restarts.
  // Set `VITE_DISABLE_WS=1` in `web/.env.local` and restart Vite.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const disable = ((import.meta as any).env?.VITE_DISABLE_WS ?? '') === '1'
  if (disable) return
  positionsWs.connect('/ws/positions')
  alertsWs.connect('/ws/alerts')
}

export function disconnectWebSockets() {
  positionsWs.disconnect()
  alertsWs.disconnect()
}
