import type { DocContent } from '@penfii/contracts'

const emptyParagraph = () => ({ type: 'paragraph', props: {}, content: [], children: [] })

function unsupportedContent(value: unknown, kind: 'node' | 'inline'): string {
  return `⟦Unsupported legacy ${kind}: ${JSON.stringify(value)}⟧`
}

function marksToStyles(marks: unknown): Record<string, unknown> {
  if (!Array.isArray(marks)) return {}
  const styles: Record<string, unknown> = {}
  for (const mark of marks) {
    if (!mark || typeof mark !== 'object') continue
    const item = mark as { type?: string; attrs?: Record<string, unknown> }
    if (['bold', 'italic', 'underline', 'strike', 'code'].includes(item.type ?? '')) styles[item.type!] = true
    if (item.type === 'textStyle' && typeof item.attrs?.color === 'string') styles.textColor = item.attrs.color
  }
  return styles
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

/** The links a saved document may hold (the server refuses any other): http(s), mailto and same-site paths. */
function safeHref(href: unknown): href is string {
  if (typeof href !== 'string') return false
  const value = href.trim()
  return /^(https?:\/\/|mailto:)/i.test(value) || (value.startsWith('/') && !value.startsWith('//'))
}

/**
 * Inline content in the Docs model. Text keeps its marks (a ProseMirror link mark becomes a Docs link when its
 * address is one a document may hold); a mention or reference without a valid id is kept as visible text.
 */
function inlineContent(value: unknown): unknown[] {
  if (!Array.isArray(value)) return []
  return value.flatMap<unknown>((node) => {
    if (!node || typeof node !== 'object') return []
    const item = node as { type?: string; text?: string; marks?: unknown; attrs?: Record<string, unknown>; content?: unknown }
    if (item.type === 'text') {
      const text = { type: 'text', text: item.text ?? '', styles: marksToStyles(item.marks) }
      const link = Array.isArray(item.marks) ? item.marks.find((mark) => (mark as { type?: string })?.type === 'link') as { attrs?: { href?: unknown } } | undefined : undefined
      return link && safeHref(link.attrs?.href) ? [{ type: 'link', href: link.attrs.href.trim(), content: [text] }] : [text]
    }
    if (item.type === 'hardBreak') return [{ type: 'text', text: '\n', styles: {} }]
    if (item.type === 'mention' && typeof item.attrs?.userId === 'string' && UUID.test(item.attrs.userId)) return [{ type: 'mention', props: { userId: item.attrs.userId } }]
    const kind = item.attrs?.kind ?? 'TICKET'
    if (item.type === 'reference' && typeof item.attrs?.id === 'string' && UUID.test(item.attrs.id) && (kind === 'TICKET' || kind === 'DOC')) return [{ type: 'reference', props: { kind, id: item.attrs.id } }]
    return [{ type: 'text', text: unsupportedContent(node, 'inline'), styles: {} }]
  })
}

/** Plain text of a node: code holds text only. */
function plainText(value: unknown): string {
  if (!value || typeof value !== 'object') return ''
  const item = value as { type?: string; text?: unknown; content?: unknown }
  if (typeof item.text === 'string') return item.text
  if (item.type === 'hardBreak') return '\n'
  return Array.isArray(item.content) ? item.content.map(plainText).join('') : ''
}

type LegacyNode = { type?: string; attrs?: Record<string, unknown>; content?: unknown }

function childNodes(node: LegacyNode): LegacyNode[] {
  return Array.isArray(node.content) ? node.content.filter((child): child is LegacyNode => Boolean(child) && typeof child === 'object') : []
}

/**
 * A list item or task item: its first paragraph is the block's text and everything after it (more paragraphs,
 * a nested list) becomes nested blocks, so nothing the item held is dropped.
 */
function itemBlock(item: LegacyNode, type: string, props: Record<string, unknown>) {
  const children = childNodes(item)
  const first = children[0]?.type === 'paragraph' ? children[0] : null
  return { type, props, content: first ? inlineContent(first.content) : [], children: (first ? children.slice(1) : children).flatMap(convertNode) }
}

function convertNode(node: unknown): unknown[] {
  if (!node || typeof node !== 'object') return []
  const item = node as LegacyNode
  switch (item.type) {
    case 'paragraph': return [{ type: 'paragraph', props: {}, content: inlineContent(item.content), children: [] }]
    case 'heading': {
      const level = Number(item.attrs?.level ?? 1)
      return [{ type: 'heading', props: { level: Number.isFinite(level) ? Math.max(1, Math.min(3, level)) : 1 }, content: inlineContent(item.content), children: [] }]
    }
    case 'codeBlock': {
      const language = typeof item.attrs?.language === 'string' && item.attrs.language ? item.attrs.language : 'text'
      const text = plainText(item)
      return [{ type: 'codeBlock', props: { language }, content: text ? [{ type: 'text', text, styles: {} }] : [], children: [] }]
    }
    case 'blockquote': {
      // Paragraphs stay one quote with their line breaks and marks; anything else in it is nested under the quote.
      const parts = childNodes(item)
      const paragraphs = parts.filter((part) => part.type === 'paragraph')
      const content = paragraphs.flatMap((part, index) => [...(index ? [{ type: 'text', text: '\n', styles: {} }] : []), ...inlineContent(part.content)])
      return [{ type: 'quote', props: {}, content, children: parts.filter((part) => part.type !== 'paragraph').flatMap(convertNode) }]
    }
    case 'bulletList': return childNodes(item).map((child) => itemBlock(child, 'bulletListItem', {}))
    case 'orderedList': return childNodes(item).map((child) => itemBlock(child, 'numberedListItem', {}))
    case 'taskList': return childNodes(item).map((child) => itemBlock(child, 'checkListItem', { checked: child.attrs?.checked === true }))
    case 'horizontalRule': return [{ type: 'divider', props: {}, content: [], children: [] }]
    case undefined:
    default:
      return [{ type: 'paragraph', props: {}, content: [{ type: 'text', text: unsupportedContent(node, 'node'), styles: {} }], children: [] }]
  }
}

/**
 * A Ticket description in the Docs model. Docs content is used as it is; the older ProseMirror JSON is converted
 * so that the result is one the server accepts as a document, and nothing in it is silently dropped: a node the
 * Docs model has no block for is kept as visible text.
 */
export function ticketDescriptionToDocContent(value: unknown): DocContent {
  if (value && typeof value === 'object') {
    const item = value as { schema?: unknown; version?: unknown; blocks?: unknown; type?: unknown; content?: unknown }
    if (item.schema === 'penfii.doc' && item.version === 1 && Array.isArray(item.blocks)) return value as DocContent
    if (item.type === 'doc' && Array.isArray(item.content)) {
      const blocks = item.content.flatMap(convertNode)
      return { schema: 'penfii.doc', version: 1, blocks: blocks.length ? blocks : [emptyParagraph()] }
    }
  }
  return { schema: 'penfii.doc', version: 1, blocks: [emptyParagraph()] }
}

/** Plain text for conflict summaries and existing compact Ticket surfaces. */
export function docContentText(content: DocContent): string {
  const blockText = (block: unknown): string => {
    if (!block || typeof block !== 'object') return ''
    const item = block as { content?: unknown; children?: unknown }
    const inline = Array.isArray(item.content) ? item.content.map((part) => {
      if (!part || typeof part !== 'object') return ''
      const value = part as { text?: unknown; content?: unknown }
      if (typeof value.text === 'string') return value.text
      return Array.isArray(value.content) ? value.content.map((child) => (child as { text?: string })?.text ?? '').join('') : ''
    }).join('') : ''
    const children = Array.isArray(item.children) ? item.children.map(blockText).filter(Boolean).join('\n') : ''
    return [inline, children].filter(Boolean).join('\n')
  }
  return content.blocks.map(blockText).filter((line) => line.length > 0).join('\n').replace(/\s+$/, '')
}
