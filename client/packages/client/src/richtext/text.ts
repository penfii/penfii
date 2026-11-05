import type { RichTextBlock, RichTextDoc, RichTextInline } from '@penfii/contracts'

function inlineText(nodes: readonly RichTextInline[] | undefined): string {
  if (!nodes) return ''
  return nodes
    .map((node) => (node.type === 'mention' ? `@${node.attrs.userId}` : node.text))
    .join('')
}

function nodeText(node: RichTextBlock | RichTextInline): string {
  if (node.type === 'text') return node.text
  if (node.type === 'mention') return `@${node.attrs.userId}`
  return blockText(node)
}

function blockText(block: RichTextBlock): string {
  switch (block.type) {
    case 'paragraph':
    case 'heading':
      return inlineText(block.content)
    case 'codeBlock':
      return (block.content ?? []).map((node) => node.text).join('')
    case 'blockquote':
      return (block.content ?? []).map((child) => nodeText(child)).join('\n')
    case 'bulletList':
    case 'orderedList':
      return (block.content ?? [])
        .map((item) => (item.content ?? []).map((child) => nodeText(child)).join('\n'))
        .join('\n')
    default:
      return ''
  }
}

export function plainText(doc: RichTextDoc): string {
  if (doc.type !== 'doc' || !Array.isArray(doc.content)) return ''
  return doc.content.map((block) => blockText(block)).join('\n')
}

export function isBlankDoc(doc: RichTextDoc): boolean {
  return plainText(doc).trim().length === 0
}
