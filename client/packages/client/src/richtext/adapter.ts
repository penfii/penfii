import type { JSONContent } from '@tiptap/core'
import type {
  RichTextBlock,
  RichTextDoc,
  RichTextInline,
  RichTextListItem,
  RichTextMark,
  RichTextText,
} from '@penfii/contracts'
import { isSafeHttpUrl } from './url'

/**
 * The editor speaks Tiptap JSON. Messages keep versioned RichText JSON.
 * Markdown never reaches the database; parseRichText is only an import.
 */

type FlowNode = RichTextBlock | RichTextInline

function isInline(node: { type?: string } | null | undefined): node is RichTextInline {
  return node?.type === 'text' || node?.type === 'mention'
}

function markFromEditor(mark: JSONContent): RichTextMark | null {
  if (mark.type === 'bold' || mark.type === 'italic' || mark.type === 'strike' || mark.type === 'code') {
    return { type: mark.type }
  }
  if (mark.type === 'link') {
    const href = typeof mark.attrs?.href === 'string' ? mark.attrs.href : ''
    if (!isSafeHttpUrl(href)) return null
    return { type: 'link', attrs: { href } }
  }
  return null
}

function marksFromEditor(marks: JSONContent[] | undefined): RichTextMark[] {
  return (marks ?? []).flatMap((mark) => {
    const converted = markFromEditor(mark)
    return converted ? [converted] : []
  })
}

function sameMarks(left: readonly RichTextMark[], right: readonly RichTextMark[]): boolean {
  return JSON.stringify(left) === JSON.stringify(right)
}

function textNode(text: string, marks: readonly RichTextMark[]): RichTextText {
  if (marks.length === 0) return { type: 'text', text }
  return { type: 'text', text, marks: [...marks] }
}

function inlinesFromEditor(nodes: readonly JSONContent[] | undefined): RichTextInline[] {
  const result: RichTextInline[] = []
  let buffer = ''
  let marks: RichTextMark[] | null = null
  const flush = () => {
    if (buffer.length === 0) return
    result.push(textNode(buffer, marks ?? []))
    buffer = ''
    marks = null
  }
  for (const node of nodes ?? []) {
    if (node.type === 'hardBreak') {
      marks ??= []
      buffer += '\n'
      continue
    }
    if (node.type === 'mention') {
      flush()
      const occurrenceId = typeof node.attrs?.occurrenceId === 'string' ? node.attrs.occurrenceId : ''
      const userId = typeof node.attrs?.userId === 'string' ? node.attrs.userId : ''
      if (occurrenceId.length > 0 && userId.length > 0) {
        result.push({ type: 'mention', attrs: { occurrenceId, kind: 'USER', userId } })
      }
      continue
    }
    if (node.type === 'text') {
      const next = marksFromEditor(node.marks)
      if (marks !== null && !sameMarks(marks, next)) flush()
      marks = next
      buffer += node.text ?? ''
      continue
    }
    flush()
  }
  flush()
  return result
}

function paragraphFromEditor(node: JSONContent): RichTextBlock {
  return { type: 'paragraph', content: inlinesFromEditor(node.content) }
}

function headingLevel(value: unknown): number {
  const level = typeof value === 'number' ? value : 1
  if (level < 1) return 1
  if (level > 6) return 6
  return level
}

function blockFromEditor(node: JSONContent): RichTextBlock | null {
  switch (node.type) {
    case 'paragraph':
      return paragraphFromEditor(node)
    case 'heading':
      return { type: 'heading', attrs: { level: headingLevel(node.attrs?.level) }, content: inlinesFromEditor(node.content) }
    case 'codeBlock': {
      const language = typeof node.attrs?.language === 'string' && node.attrs.language.length > 0 ? node.attrs.language : null
      const text = (node.content ?? []).map((child) => child.text ?? '').join('')
      return {
        type: 'codeBlock',
        attrs: { language },
        content: text.length > 0 ? [{ type: 'text', text }] : [],
      }
    }
    case 'blockquote':
      return { type: 'blockquote', content: blocksFromEditor(node.content) }
    case 'bulletList':
      return { type: 'bulletList', content: listItemsFromEditor(node.content) }
    case 'orderedList': {
      const start = typeof node.attrs?.start === 'number' ? node.attrs.start : 1
      const content = listItemsFromEditor(node.content)
      if (start > 1) return { type: 'orderedList', attrs: { start }, content }
      return { type: 'orderedList', content }
    }
    default: {
      const text = plainPieces(node).join('')
      if (text.length === 0) return null
      return { type: 'paragraph', content: [{ type: 'text', text }] }
    }
  }
}

function plainPieces(node: JSONContent): string[] {
  if (node.type === 'text') return [node.text ?? '']
  return (node.content ?? []).flatMap((child) => plainPieces(child))
}

function blocksFromEditor(nodes: readonly JSONContent[] | undefined): RichTextBlock[] {
  return (nodes ?? []).flatMap((node) => {
    const block = blockFromEditor(node)
    return block ? [block] : []
  })
}

function listItemsFromEditor(nodes: readonly JSONContent[] | undefined): RichTextListItem[] {
  return (nodes ?? []).flatMap((node) => {
    if (node.type !== 'listItem') return []
    const content = blocksFromEditor(node.content)
    return [{ type: 'listItem', content: content.length > 0 ? content : [{ type: 'paragraph', content: [] }] }]
  })
}

function marksToEditor(marks: readonly RichTextMark[] | undefined): JSONContent[] {
  const result: JSONContent[] = []
  for (const mark of marks ?? []) {
    if (mark.type === 'link') {
      if (isSafeHttpUrl(mark.attrs.href)) result.push({ type: 'link', attrs: { href: mark.attrs.href } })
      continue
    }
    result.push({ type: mark.type })
  }
  return result
}

function inlinesToEditor(nodes: readonly RichTextInline[] | undefined): JSONContent[] {
  const result: JSONContent[] = []
  for (const node of nodes ?? []) {
    if (node.type === 'mention') {
      result.push({
        type: 'mention',
        attrs: { occurrenceId: node.attrs.occurrenceId, kind: 'USER', userId: node.attrs.userId },
      })
      continue
    }
    const parts = node.text.split('\n')
    const marks = marksToEditor(node.marks)
    parts.forEach((part, index) => {
      if (index > 0) result.push({ type: 'hardBreak' })
      if (part.length === 0) return
      const text: JSONContent = { type: 'text', text: part }
      if (marks.length > 0) text.marks = marks.map((mark) => ({ ...mark, type: mark.type ?? 'text' }))
      result.push(text)
    })
  }
  return result
}

function flowToEditor(nodes: readonly FlowNode[] | undefined): JSONContent[] {
  const content = nodes ?? []
  if (content.length === 0) return [{ type: 'paragraph' }]
  if (content.every((node) => isInline(node))) {
    return [{ type: 'paragraph', content: inlinesToEditor(content) }]
  }
  return content.flatMap((node) => {
    if (isInline(node)) return [{ type: 'paragraph', content: inlinesToEditor([node]) }]
    const block = blockToEditor(node)
    return block ? [block] : []
  })
}

function blockToEditor(block: RichTextBlock): JSONContent | null {
  switch (block.type) {
    case 'paragraph':
      return { type: 'paragraph', content: inlinesToEditor(block.content) }
    case 'heading':
      return {
        type: 'heading',
        attrs: { level: headingLevel(block.attrs?.level) },
        content: inlinesToEditor(block.content),
      }
    case 'codeBlock': {
      const language = block.attrs?.language
      const text = (block.content ?? []).map((node) => node.text).join('')
      return {
        type: 'codeBlock',
        attrs: { language: language && language.length > 0 ? language : null },
        content: text.length > 0 ? [{ type: 'text', text }] : [],
      }
    }
    case 'blockquote':
      return { type: 'blockquote', content: flowToEditor(block.content as FlowNode[] | undefined) }
    case 'bulletList':
      return { type: 'bulletList', content: (block.content ?? []).map(listItemToEditor) }
    case 'orderedList': {
      const start = block.attrs?.start ?? 1
      return {
        type: 'orderedList',
        attrs: { start: start > 1 ? start : 1 },
        content: (block.content ?? []).map(listItemToEditor),
      }
    }
    default:
      return null
  }
}

function listItemToEditor(item: RichTextListItem): JSONContent {
  return { type: 'listItem', content: flowToEditor(item.content as FlowNode[] | undefined) }
}

export function richTextToEditor(doc: RichTextDoc): JSONContent {
  const content = (doc.content ?? []).flatMap((block) => {
    const converted = blockToEditor(block)
    return converted ? [converted] : []
  })
  return { type: 'doc', content: content.length > 0 ? content : [{ type: 'paragraph' }] }
}

export function editorToRichText(json: JSONContent, baseline?: RichTextDoc | null): RichTextDoc {
  const content = blocksFromEditor(json.content)
  const doc: RichTextDoc = {
    type: 'doc',
    content: content.length > 0 ? content : [{ type: 'paragraph', content: [] }],
  }
  if (baseline && sameRichText(doc, baseline)) return baseline
  return doc
}

function canonicalMark(mark: RichTextMark): RichTextMark {
  if (mark.type === 'link') return { type: 'link', attrs: { href: mark.attrs.href } }
  return { type: mark.type }
}

function canonicalInline(node: RichTextInline): RichTextInline {
  if (node.type === 'mention') {
    return {
      type: 'mention',
      attrs: { occurrenceId: node.attrs.occurrenceId, kind: 'USER', userId: node.attrs.userId },
    }
  }
  const marks = (node.marks ?? [])
    .map(canonicalMark)
    .sort((left, right) => left.type.localeCompare(right.type))
  return textNode(node.text, marks)
}

function canonicalFlow(nodes: readonly FlowNode[] | undefined): RichTextBlock[] {
  const content = nodes ?? []
  if (content.length === 0) return [{ type: 'paragraph', content: [] }]
  if (content.every((node) => isInline(node))) {
    return [{ type: 'paragraph', content: content.map(canonicalInline) }]
  }
  return content.flatMap((node) => (isInline(node) ? [{ type: 'paragraph' as const, content: [canonicalInline(node)] }] : canonicalBlock(node)))
}

function canonicalBlock(block: RichTextBlock): RichTextBlock {
  switch (block.type) {
    case 'paragraph':
      return { type: 'paragraph', content: (block.content ?? []).map(canonicalInline) }
    case 'heading':
      return {
        type: 'heading',
        attrs: { level: headingLevel(block.attrs?.level) },
        content: (block.content ?? []).map(canonicalInline),
      }
    case 'codeBlock': {
      const language = block.attrs?.language
      return {
        type: 'codeBlock',
        attrs: { language: language && language.length > 0 ? language : null },
        content: [{ type: 'text', text: (block.content ?? []).map((node) => node.text).join('') }],
      }
    }
    case 'blockquote':
      return { type: 'blockquote', content: canonicalFlow(block.content as FlowNode[] | undefined) }
    case 'bulletList':
      return {
        type: 'bulletList',
        content: (block.content ?? []).map((item) => ({
          type: 'listItem' as const,
          content: canonicalFlow(item.content as FlowNode[] | undefined),
        })),
      }
    case 'orderedList':
      return {
        type: 'orderedList',
        attrs: { start: block.attrs?.start && block.attrs.start > 1 ? block.attrs.start : 1 },
        content: (block.content ?? []).map((item) => ({
          type: 'listItem' as const,
          content: canonicalFlow(item.content as FlowNode[] | undefined),
        })),
      }
    default:
      return { type: 'paragraph', content: [] }
  }
}

export function sameRichText(left: RichTextDoc, right: RichTextDoc): boolean {
  const a = { type: 'doc', content: (left.content ?? []).map(canonicalBlock) }
  const b = { type: 'doc', content: (right.content ?? []).map(canonicalBlock) }
  return JSON.stringify(a) === JSON.stringify(b)
}
