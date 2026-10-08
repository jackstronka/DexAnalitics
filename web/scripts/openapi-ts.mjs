/**
 * C2: generate / check `src/lib/api.gen.ts` from `crates/api/openapi.json`.
 *   npm run gen:api       — write
 *   npm run check:api-gen — fail if committed file drifted
 */
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import openapiTS, { astToString, COMMENT_HEADER } from 'openapi-typescript'

const webDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const specPath = path.resolve(webDir, '../crates/api/openapi.json')
const outPath = path.join(webDir, 'src/lib/api.gen.ts')

function normalizeNl(text) {
  return text.replace(/\r\n/g, '\n')
}

async function generate() {
  const spec = JSON.parse(fs.readFileSync(specPath, 'utf8'))
  const ast = await openapiTS(spec)
  let body = `${COMMENT_HEADER}${astToString(ast)}`
  if (!body.endsWith('\n')) body += '\n'
  return normalizeNl(body)
}

const text = await generate()
const check = process.argv.includes('--check')
if (check) {
  if (!fs.existsSync(outPath)) {
    console.error(`missing ${outPath}\nRegenerate with: cd web && npm run gen:api`)
    process.exit(1)
  }
  const committed = normalizeNl(fs.readFileSync(outPath, 'utf8'))
  if (committed !== text) {
    console.error('web/src/lib/api.gen.ts is stale vs crates/api/openapi.json')
    console.error('Regenerate with: cd web && npm run gen:api')
    process.exit(1)
  }
  console.log('openapi-ts check: api.gen.ts matches crates/api/openapi.json')
} else {
  fs.writeFileSync(outPath, text, 'utf8')
  console.log(`wrote ${path.relative(webDir, outPath).replaceAll('\\', '/')}`)
}
