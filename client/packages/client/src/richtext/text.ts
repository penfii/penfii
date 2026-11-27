import type { RichTextBlock, RichTextDoc, RichTextInline } from '@penfii/contracts'

/** A person's name for a mention, when it is known; otherwise the text shows the id. */
export type MentionName = (userId: string) => string | undefined

const mentionText = (userId: string, nameOf: MentionName | undefined) => `@${nameOf?.(userId) ?? userId}`

function inlineText(nodes: readonly RichTextInline[] | undefined, nameOf?: MentionName): string {
  if (!nodes) return ''
  return nodes
    .map((node) => (node.type === 'mention' ? mentionText(node.attrs.userId, nameOf) : node.text))
    .join('')
}

function nodeText(node: RichTextBlock | RichTextInline, nameOf?: MentionName): string {
  if (node.type === 'text') return node.text
  if (node.type === 'mention') return mentionText(node.attrs.userId, nameOf)
  return blockText(node, nameOf)
}

function blockText(block: RichTextBlock, nameOf?: MentionName): string {
  switch (block.type) {
    case 'paragraph':
    case 'heading':
      return inlineText(block.content, nameOf)
    case 'codeBlock':
      return (block.content ?? []).map((node) => node.text).join('')
    case 'blockquote':
      return (block.content ?? []).map((child) => nodeText(child, nameOf)).join('\n')
    case 'bulletList':
    case 'orderedList':
      return (block.content ?? [])
        .map((item) => (item.content ?? []).map((child) => nodeText(child, nameOf)).join('\n'))
        .join('\n')
    default:
      return ''
  }
}

/** The text of a document. With `nameOf`, a mention reads `@Name` instead of `@<id>`. */
export function plainText(doc: RichTextDoc, nameOf?: MentionName): string {
  if (doc.type !== 'doc' || !Array.isArray(doc.content)) return ''
  return doc.content.map((block) => blockText(block, nameOf)).join('\n')
}

/** The ids of the people a document mentions. */
export function mentionedUserIds(doc: RichTextDoc): string[] {
  const ids = new Set<string>()
  const visit = (node: unknown) => {
    if (typeof node !== 'object' || node === null) return
    const record = node as { type?: string; attrs?: { userId?: string }; content?: unknown[] }
    if (record.type === 'mention' && record.attrs?.userId) ids.add(record.attrs.userId)
    for (const child of record.content ?? []) visit(child)
  }
  visit(doc)
  return [...ids]
}

export function isBlankDoc(doc: RichTextDoc): boolean {
  return plainText(doc).trim().length === 0
}

const BLOCKS = new Set(['paragraph', 'heading', 'blockquote', 'bulletList', 'orderedList', 'listItem', 'codeBlock'])
const MARKS = new Set(['bold', 'italic', 'strike', 'code', 'link'])

/**
 * What a stored body is, as far as the editor and `RichTextView` are concerned: `shape` is true when every node of it is
 * one they carry whole (so nothing would be dropped by showing it or by editing it), and `formatted` when it also has
 * more than plain paragraphs and mentions (a heading, a list, a quote, code, or marked text).
 */
function inspectRichText(value: unknown): { shape: boolean; formatted: boolean } {
  if (!value || typeof value !== 'object') return { shape: false, formatted: false }
  const doc = value as { type?: unknown; content?: unknown }
  if (doc.type !== 'doc' || !Array.isArray(doc.content)) return { shape: false, formatted: false }
  let formatted = false
  const walk = (node: unknown, depth: number): boolean => {
    if (depth > 20 || !node || typeof node !== 'object') return false
    const item = node as { type?: unknown; content?: unknown; marks?: unknown }
    if (item.type === 'text') {
      if (item.marks !== undefined) {
        if (!Array.isArray(item.marks) || !item.marks.every((mark) => MARKS.has((mark as { type?: string })?.type ?? ''))) return false
        if (item.marks.length > 0) formatted = true
      }
      return true
    }
    if (item.type === 'mention') return true
    if (typeof item.type !== 'string' || !BLOCKS.has(item.type)) return false
    if (item.type !== 'paragraph' && item.type !== 'listItem') formatted = true
    if (item.content === undefined) return true
    return Array.isArray(item.content) && item.content.every((child) => walk(child, depth + 1))
  }
  return { shape: doc.content.every((block) => walk(block, 0)), formatted }
}

/**
 * Whether a stored body is chat-style rich text that `RichTextView` can show whole, and has formatting in it (a heading, a
 * list, a quote, code, or marked text) rather than only plain paragraphs. A body of any other shape, or one with a node the
 * view would drop, is not: the caller shows its plain text instead, so nothing is ever hidden.
 */
export function isFormattedRichText(value: unknown): value is RichTextDoc {
  const found = inspectRichText(value)
  return found.shape && found.formatted
}

/**
 * Whether a stored body is chat-style rich text that the editor can start from as it is: plain paragraphs, formatted text
 * and mentions, every node of which it keeps. Anything else (a Doc-format body, a node of a kind it does not know) is not,
 * and is edited as its plain text.
 */
export function isEditableRichText(value: unknown): value is RichTextDoc {
  return inspectRichText(value).shape
}
