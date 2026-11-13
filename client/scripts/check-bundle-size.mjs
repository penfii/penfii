// Fails when the JavaScript a visitor must download before the first screen grows past its budget, or when a
// library that belongs to one screen is pulled into every page again. Run it after `npm run build`.
//
// "Initial" is what index.html asks for at once: the entry script and every modulepreload. The texts of
// the language the app opens in are fetched by the entry right away, so the larger of the two catalogs is
// counted on top as the worst case. Everything else (editors, highlighter, other languages, pages) loads when used.
//
// Budgets sit about 15% above the measured size (2026-11-13: initial 689,795 B raw / 200,854 B gzip; the larger
// catalog 38,090 B gzip). Raise them on purpose, with the reason in the commit; do not let them creep.
import { readFileSync, readdirSync } from 'node:fs'
import { gzipSync } from 'node:zlib'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const dist = join(dirname(fileURLToPath(import.meta.url)), '..', 'apps', 'web', 'dist')
const BUDGET = { initialRaw: 800_000, initialGzip: 235_000, withCatalogGzip: 275_000 }
// Code that must not be in the first load. Each pattern is a string that library carries in its own source.
const KEEP_OUT = {
  'tiptap / ProseMirror (message and ticket editor)': /prosemirror/i,
  'shiki (code highlighter)': /createHighlighterCore/,
  'BlockNote (doc editor)': /blocknote/i,
}

const html = readFileSync(join(dist, 'index.html'), 'utf8')
const initial = [...new Set([...html.matchAll(/(?:src|href)="\/(assets\/[^"]+\.js)"/g)].map((match) => match[1]))]
if (initial.length === 0) {
  console.error('bundle budget: index.html names no script; was the web app built (npm run build)?')
  process.exit(2)
}

const size = (file) => {
  const bytes = readFileSync(join(dist, file))
  return { raw: bytes.length, gzip: gzipSync(bytes).length }
}
let raw = 0
let gzip = 0
const problems = []
for (const file of initial) {
  const own = size(file)
  raw += own.raw
  gzip += own.gzip
  const text = readFileSync(join(dist, file), 'utf8')
  for (const [name, pattern] of Object.entries(KEEP_OUT)) {
    if (pattern.test(text)) problems.push(`${name} is in the first load (${file})`)
  }
}
const catalogs = readdirSync(join(dist, 'assets')).filter((file) => /^messages\.(ko|en)-.*\.js$/.test(file)).map((file) => size(`assets/${file}`).gzip)
if (catalogs.length !== 2) problems.push(`expected two language catalogs as their own chunks, found ${catalogs.length}`)
const catalog = Math.max(0, ...catalogs)

const kb = (bytes) => `${(bytes / 1000).toFixed(1)} kB`
console.log(`initial JS: ${initial.length} files, ${raw.toLocaleString('en-US')} B raw, ${gzip.toLocaleString('en-US')} B gzip (+ largest catalog ${catalog.toLocaleString('en-US')} B gzip = ${kb(gzip + catalog)})`)
if (raw > BUDGET.initialRaw) problems.push(`initial raw ${raw} B is over the budget of ${BUDGET.initialRaw} B`)
if (gzip > BUDGET.initialGzip) problems.push(`initial gzip ${gzip} B is over the budget of ${BUDGET.initialGzip} B`)
if (gzip + catalog > BUDGET.withCatalogGzip) problems.push(`initial gzip with a catalog ${gzip + catalog} B is over the budget of ${BUDGET.withCatalogGzip} B`)
if (problems.length > 0) {
  console.error(`bundle budget FAILED:\n- ${problems.join('\n- ')}`)
  process.exit(1)
}
console.log('bundle budget: OK')
