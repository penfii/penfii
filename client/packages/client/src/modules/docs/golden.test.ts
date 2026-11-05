import { createRequire } from 'node:module'
import { describe, expect, it } from 'vitest'
import * as Y from 'yjs'
import { BlockNoteEditor } from '@blocknote/core'
import { blocksToYXmlFragment, yXmlFragmentToBlocks } from '@blocknote/core/yjs'
import { GOLDEN_CASES } from './golden'
import { docSchema } from './schema'
import golden from './doc-golden.json'

/**
 * Golden fixtures: BlockNote's view of the Yjs bytes and the Rust server's view must be the same.
 *
 *   UPDATE_GOLDEN=1 npx vitest run docs/golden   regenerates `bn_blocks` and `update_js` here;
 *   then copy this file to server/crates/core/fixtures/doc-golden.json and run
 *   UPDATE_GOLDEN=1 cargo test -p penfii-core golden   there, which fills `update_rust`; copy it back.
 *
 * Without UPDATE_GOLDEN this test fails when BlockNote (after an upgrade) produces other bytes or other
 * blocks than the checked-in file, or when it cannot read what the Rust seeder wrote.
 */
const JS_CLIENT_ID = 0x5eed0002
const require = createRequire(import.meta.url)

type Fixture = { name: string; input: unknown[]; bn_blocks: unknown; update_js: string; update_rust: string }

const b64 = (bytes: Uint8Array) => Buffer.from(bytes).toString('base64')
const fromB64 = (text: string) => new Uint8Array(Buffer.from(text, 'base64'))
const editor = BlockNoteEditor.create({ schema: docSchema })

function readBlocks(update: Uint8Array): unknown {
  const doc = new Y.Doc()
  Y.applyUpdate(doc, update)
  // Through JSON, as the fixture file holds it: `undefined` becomes `null` or disappears there.
  return JSON.parse(JSON.stringify(yXmlFragmentToBlocks(editor, doc.getXmlFragment('document-store'))))
}

describe('doc golden fixtures', () => {
  const existing = golden as unknown as { fixtures: Fixture[] }

  it('BlockNote makes the checked-in bytes and blocks from the cases', () => {
    const fixtures: Fixture[] = GOLDEN_CASES.map((c) => {
      const doc = new Y.Doc()
      doc.clientID = JS_CLIENT_ID
      blocksToYXmlFragment(editor, c.blocks as never, doc.getXmlFragment('document-store'))
      const update = Y.encodeStateAsUpdate(doc)
      const previous = existing.fixtures.find((f) => f.name === c.name)
      return {
        name: c.name,
        input: c.blocks,
        bn_blocks: readBlocks(update),
        update_js: b64(update),
        update_rust: previous?.update_rust ?? '',
      }
    })
    if (process.env.UPDATE_GOLDEN === '1') {
      const text = JSON.stringify({ blocknote: '0.56.0', yjs: '13.6.33', fixtures }, null, 1) + '\n'
      require('node:fs').writeFileSync(require('node:path').join(import.meta.dirname, 'doc-golden.json'), text)
      return
    }
    expect(existing.fixtures.map((f) => f.name)).toEqual(GOLDEN_CASES.map((c) => c.name))
    for (const [i, f] of fixtures.entries()) {
      expect(f.update_js, `${f.name}: bytes`).toBe(existing.fixtures[i]!.update_js)
      expect(f.bn_blocks, `${f.name}: blocks`).toEqual(existing.fixtures[i]!.bn_blocks)
    }
  })

  it('BlockNote reads what the Rust seeder wrote as the same blocks', () => {
    for (const f of existing.fixtures) {
      expect(f.update_rust, `${f.name}: the Rust seed is missing; run the Rust half of the update`).not.toBe('')
      expect(readBlocks(fromB64(f.update_rust)), f.name).toEqual(f.bn_blocks)
    }
  })
})
