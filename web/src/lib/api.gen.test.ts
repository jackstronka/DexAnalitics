import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import type { OkJson, Schema } from '@/lib/api.contract'

const here = path.dirname(fileURLToPath(import.meta.url))

type HealthFromSchema = Schema<'HealthResponse'>
type HealthFromPath = OkJson<'/health', 'get'>
type DipSnapshotFromSchema = Schema<'DipSnapshotResponse'>
type DipSnapshotFromPath = OkJson<'/dip/snapshot', 'get'>

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

function sampleDipSnapshot(): DipSnapshotFromSchema {
  return {
    symbol: 'SOL',
    mint: 'So11111111111111111111111111111111111111112',
    zone: 'none',
    zones: {
      a: { min: 100, max: 200 },
      b: { min: 50, max: 99.99 },
      c: { min: 0, max: 49.99 },
    },
    price_usd: 150,
    price_source: 'jupiter',
    buy_dip_orders: [],
    orders_source: 'no_jwt',
    notes: [],
    fetched_at: '2026-11-09T00:00:00Z',
  }
}

function assertAssigns(_value: HealthFromPath): void {
  /* compile-time: path 200 body === HealthResponse schema */
}

function assertDipAssigns(_value: DipSnapshotFromPath): void {
  /* compile-time: path 200 body === DipSnapshotResponse schema */
}

describe('api.gen OpenAPI types (C2)', () => {
  it('HealthResponse sample matches the generated schema', () => {
    const health = sampleHealth()
    assertAssigns(health)
    expect(health.status).toBe('healthy')
    expect(health.components.rpc).toBe(true)
    expect(health.components.circuit_breaker).toBe('closed')
  })

  it('DipSnapshotResponse sample matches the generated schema', () => {
    const snap = sampleDipSnapshot()
    assertDipAssigns(snap)
    expect(snap.orders_source).toBe('no_jwt')
    expect(snap.buy_dip_orders).toEqual([])
    expect(snap.zones.a.min).toBe(100)
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

  it('committed gen file lists /dip/snapshot and DipSnapshotResponse', () => {
    const gen = readFileSync(path.join(here, 'api.gen.ts'), 'utf8')
    const spec = JSON.parse(
      readFileSync(path.resolve(here, '../../../crates/api/openapi.json'), 'utf8'),
    ) as { paths: Record<string, unknown>; components: { schemas: Record<string, unknown> } }
    expect(spec.paths['/dip/snapshot']).toBeTruthy()
    expect(spec.components.schemas.DipSnapshotResponse).toBeTruthy()
    expect(gen).toContain('"/dip/snapshot"')
    expect(gen).toContain('DipSnapshotResponse')
  })

  it('committed gen file lists /dip/runtime and DipRuntimeResponse', () => {
    const gen = readFileSync(path.join(here, 'api.gen.ts'), 'utf8')
    const spec = JSON.parse(
      readFileSync(path.resolve(here, '../../../crates/api/openapi.json'), 'utf8'),
    ) as { paths: Record<string, unknown>; components: { schemas: Record<string, unknown> } }
    expect(spec.paths['/dip/runtime']).toBeTruthy()
    expect(spec.components.schemas.DipRuntimeResponse).toBeTruthy()
    expect(gen).toContain('"/dip/runtime"')
    expect(gen).toContain('DipRuntimeResponse')
  })
})
