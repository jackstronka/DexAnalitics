import { Component, type ReactNode } from 'react'
import { Link } from 'react-router-dom'
import { Button } from '@/components/ui/button'

type Props = {
  title?: string
  children: ReactNode
}

type State = {
  error?: Error
}

export default class PageErrorBoundary extends Component<Props, State> {
  state: State = {}

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  componentDidCatch(error: Error, info: unknown) {
    // Keep it simple: React will show this fallback; dev console still has stack traces.
    // eslint-disable-next-line no-console
    console.error('PageErrorBoundary caught error', error, info)
  }

  render() {
    if (!this.state.error) {
      return this.props.children
    }

    const title = this.props.title ?? 'Page crashed while rendering'
    const msg = this.state.error?.message || String(this.state.error)

    return (
      <div className="max-w-3xl mx-auto px-4 py-10 space-y-4">
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-4 py-3">
          <div className="font-medium text-destructive">{title}</div>
          <div className="mt-2 text-sm text-muted-foreground break-words font-mono">{msg}</div>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" onClick={() => window.location.reload()}>
            Reload page
          </Button>
          <Link to="/positions">
            <Button variant="outline">Back to positions</Button>
          </Link>
          <Link to="/logs">
            <Button variant="outline">Open logs</Button>
          </Link>
        </div>
        <div className="text-xs text-muted-foreground">
          If this happens again, open DevTools Console and paste the error stack — it usually points to the exact field
          that had unexpected data.
        </div>
      </div>
    )
  }
}

