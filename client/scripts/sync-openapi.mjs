#!/usr/bin/env node
// packages/contracts/openapi.json is a copy of the server's api/openapi.json, which is the public
// description of the API. The contract test (packages/contracts/src/openapi.test.ts) holds the client
// types to this copy.
//
//   node scripts/sync-openapi.mjs            copy the server's file here
//   node scripts/sync-openapi.mjs --check    fail when the copy differs from the server's file
//
// The server's file is `$PENFII_OPENAPI` (a path or an https:// URL), else `../server/api/openapi.json`.
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..')
const copy = path.join(root, 'packages', 'contracts', 'openapi.json')
const location = process.env.PENFII_OPENAPI || path.join(root, '..', 'server', 'api', 'openapi.json')

async function original() {
  if (/^https?:\/\//.test(location)) {
    const response = await fetch(location)
    if (!response.ok) throw new Error(`${location}: HTTP ${response.status}`)
    return Buffer.from(await response.arrayBuffer())
  }
  return fs.readFileSync(location)
}

const wanted = await original().catch((error) => {
  console.error(`Cannot read the server's openapi.json (${location}): ${error.message}`)
  console.error('Set PENFII_OPENAPI to its path or URL.')
  process.exit(2)
})
const current = fs.existsSync(copy) ? fs.readFileSync(copy) : null

if (process.argv.includes('--check')) {
  if (current && current.equals(wanted)) {
    console.log('packages/contracts/openapi.json matches the server.')
  } else {
    console.error("packages/contracts/openapi.json differs from the server's openapi.json. Run: npm run contract:sync")
    process.exit(1)
  }
} else {
  fs.writeFileSync(copy, wanted)
  console.log(current && current.equals(wanted) ? 'Already up to date.' : 'Updated packages/contracts/openapi.json.')
}
