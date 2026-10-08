/**
 * OpenAPI-derived types for the web client (C2).
 *
 * New API functions must type request/response from `paths` / `components`
 * (or helpers below). Do not add parallel handwritten interfaces in `api.ts`
 * for new endpoints; migrate existing ones when that call is touched.
 */
import type { components, paths } from './api.gen'

export type { components, paths }

export type Schema<N extends keyof components['schemas']> = components['schemas'][N]

/** JSON body of a 200 `application/json` response, when the operation declares one. */
export type OkJson<P extends keyof paths, M extends keyof paths[P]> = paths[P][M] extends {
  responses: { 200: { content: { 'application/json': infer R } } }
}
  ? R
  : never
