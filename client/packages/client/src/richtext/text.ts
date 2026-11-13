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
