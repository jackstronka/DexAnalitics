import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import type { OkJson, Schema } from '@/lib/api.contract'

const here = path.dirname(fileURLToPath(import.meta.url))

type HealthFromSchema = Schema<'HealthResponse'>
type HealthFromPath = OkJson<'/health', 'get'>

function sampleHealth(): HealthFromSchema {
  return {
    status: 'healthy',
    version: 'test',
    uptime_secs: 1,
    components: {
      rpc: true,
      database: true,
      circuit_breaker: 'closed',
    },
  }
}

function assertAssigns(_value: HealthFromPath): void {
  /* compile-time: path 200 body === HealthResponse schema */
}

describe('api.gen OpenAPI types (C2)', () => {
  it('HealthResponse sample matches the generated schema', () => {
    const health = sampleHealth()
    assertAssigns(health)
    expect(health.status).toBe('healthy')
    expect(health.components.rpc).toBe(true)
    expect(health.components.circuit_breaker).toBe('closed')
  })

  it('committed gen file lists /health from the OpenAPI snapshot', () => {
    const gen = readFileSync(path.join(here, 'api.gen.ts'), 'utf8')
    const spec = JSON.parse(
      readFileSync(path.resolve(here, '../../../crates/api/openapi.json'), 'utf8'),
    ) as { paths: Record<string, unknown>; components: { schemas: Record<string, unknown> } }
    expect(spec.paths['/health']).toBeTruthy()
    expect(spec.components.schemas.HealthResponse).toBeTruthy()
    expect(gen).toContain('"/health"')
    expect(gen).toContain('HealthResponse')
  })
})
