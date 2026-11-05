import type { DocContent } from '@penfii/contracts'

/**
 * Between the saved document (DocContent) and what the editor holds.
 *
 * The editor edits a draft in the browser; nothing reaches the server until Save. These are the two
 * conversions around it. The server sanitises whatever Save sends, so `toContent` only wraps.
 */

function emptyBlocks(): unknown[] {
  return [{ type: 'paragraph', props: {}, content: [], children: [] }]
}

/**
 * The editor refuses a text run that is `code` and anything else at once (a ProseMirror rule) and
 * fails to load the whole page for it. The editor itself never produces such a run, but the server
 * accepts it (API-made content, templates). Code wins, as it does when code is switched on in the editor.
 */
function codeAlone(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(codeAlone)
  if (value === null || typeof value !== 'object') return value
  const node = value as Record<string, unknown>
  const out: Record<string, unknown> = {}
  for (const [key, child] of Object.entries(node)) out[key] = codeAlone(child)
  const styles = node.styles
  if (node.type === 'text' && styles && typeof styles === 'object' && (styles as Record<string, unknown>).code) {
    out.styles = { code: true }
  }
  return out
}

/** The blocks the editor starts from. */
export function toBlocks(content: DocContent): unknown[] {
  const blocks = codeAlone(content.blocks)
  return Array.isArray(blocks) && blocks.length > 0 ? blocks : emptyBlocks()
}

export function toContent(blocks: unknown[]): DocContent {
  return { schema: 'penfii.doc', version: 1, blocks }
}
