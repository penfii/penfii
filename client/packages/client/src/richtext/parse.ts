import type { RichTextBlock, RichTextDoc, RichTextInline, RichTextMark, RichTextText } from '@penfii/contracts'
import { isSafeHttpUrl } from './url'

function textNode(text: string, marks: RichTextMark[]): RichTextText {
  if (marks.length === 0) return { type: 'text', text }
  return { type: 'text', text, marks }
}

export function parseInline(input: string): RichTextInline[] {
  const nodes: RichTextInline[] = []
  let buffer = ''
  let index = 0

  const flush = () => {
    if (buffer.length === 0) return
    nodes.push({ type: 'text', text: buffer })
    buffer = ''
  }

  while (index < input.length) {
    if (input.startsWith('```', index)) {
      buffer += '`'
      index += 1
      continue
    }
    if (input[index] === '`') {
      const end = input.indexOf('`', index + 1)
      if (end > index) {
        flush()
        nodes.push(textNode(input.slice(index + 1, end), [{ type: 'code' }]))
        index = end + 1
        continue
      }
    }
    if (input.startsWith('**', index)) {
      const end = input.indexOf('**', index + 2)
      if (end !== -1) {
        flush()
        for (const node of parseInline(input.slice(index + 2, end))) {
          if (node.type === 'text') nodes.push(textNode(node.text, [...(node.marks ?? []), { type: 'bold' }]))
          else nodes.push(node)
        }
        index = end + 2
        continue
      }
    }
    if (input.startsWith('~~', index)) {
      const end = input.indexOf('~~', index + 2)
      if (end !== -1) {
        flush()
        for (const node of parseInline(input.slice(index + 2, end))) {
          if (node.type === 'text') nodes.push(textNode(node.text, [...(node.marks ?? []), { type: 'strike' }]))
          else nodes.push(node)
        }
        index = end + 2
        continue
      }
    }
    if (input[index] === '*' && input[index + 1] !== '*') {
      const end = input.indexOf('*', index + 1)
      if (end !== -1 && input[end + 1] !== '*') {
        flush()
        for (const node of parseInline(input.slice(index + 1, end))) {
          if (node.type === 'text') nodes.push(textNode(node.text, [...(node.marks ?? []), { type: 'italic' }]))
          else nodes.push(node)
        }
        index = end + 1
        continue
      }
    }
    if (input[index] === '[') {
      const labelEnd = input.indexOf(']', index + 1)
      if (labelEnd !== -1 && input[labelEnd + 1] === '(') {
        const hrefEnd = input.indexOf(')', labelEnd + 2)
        if (hrefEnd !== -1) {
          const label = input.slice(index + 1, labelEnd)
          const href = input.slice(labelEnd + 2, hrefEnd)
          flush()
          if (isSafeHttpUrl(href)) nodes.push(textNode(label, [{ type: 'link', attrs: { href } }]))
          else buffer += input.slice(index, hrefEnd + 1)
          index = hrefEnd + 1
          continue
        }
      }
    }
    buffer += input[index] ?? ''
    index += 1
  }
  flush()
  return nodes
}

function isStructural(line: string): boolean {
  return (
    line.startsWith('```') ||
    /^#{1,6} /.test(line) ||
    line.startsWith('> ') ||
    /^[-*] /.test(line) ||
    /^\d+\. /.test(line)
  )
}

export function parseRichText(source: string): RichTextDoc {
  const lines = source.replace(/\r\n/g, '\n').split('\n')
  const blocks: RichTextBlock[] = []
  let index = 0
  while (index < lines.length) {
    const line = lines[index] ?? ''
    if (line.trim() === '') {
      index += 1
      continue
    }
    if (line.startsWith('```')) {
      const language = line.slice(3).trim()
      const code: string[] = []
      index += 1
      while (index < lines.length && !(lines[index] ?? '').startsWith('```')) {
        code.push(lines[index] ?? '')
        index += 1
      }
      if (index < lines.length) index += 1
      const text = code.join('\n')
      blocks.push({
        type: 'codeBlock',
        attrs: { language: language.length > 0 ? language : null },
        content: text.length > 0 ? [{ type: 'text', text }] : [],
      })
      continue
    }
    const heading = /^(#{1,6}) (.*)$/.exec(line)
    if (heading) {
      blocks.push({
        type: 'heading',
        attrs: { level: heading[1]?.length ?? 1 },
        content: parseInline(heading[2] ?? ''),
      })
      index += 1
      continue
    }
    if (line.startsWith('> ')) {
      const quoted: string[] = []
      while (index < lines.length && (lines[index] ?? '').startsWith('> ')) {
        quoted.push((lines[index] ?? '').slice(2))
        index += 1
      }
      blocks.push({ type: 'blockquote', content: [{ type: 'paragraph', content: parseInline(quoted.join('\n')) }] })
      continue
    }
    if (/^[-*] /.test(line)) {
      const items: Array<{ type: 'listItem'; content: RichTextBlock[] }> = []
      while (index < lines.length && /^[-*] /.test(lines[index] ?? '')) {
        items.push({
          type: 'listItem',
          content: [{ type: 'paragraph', content: parseInline((lines[index] ?? '').replace(/^[-*] /, '')) }],
        })
        index += 1
      }
      blocks.push({ type: 'bulletList', content: items })
      continue
    }
    if (/^\d+\. /.test(line)) {
      const items: Array<{ type: 'listItem'; content: RichTextBlock[] }> = []
      while (index < lines.length && /^\d+\. /.test(lines[index] ?? '')) {
        items.push({
          type: 'listItem',
          content: [{ type: 'paragraph', content: parseInline((lines[index] ?? '').replace(/^\d+\. /, '')) }],
        })
        index += 1
      }
      blocks.push({ type: 'orderedList', content: items })
      continue
    }
    const paragraph: string[] = []
    while (index < lines.length && (lines[index] ?? '').trim() !== '' && !isStructural(lines[index] ?? '')) {
      paragraph.push(lines[index] ?? '')
      index += 1
    }
    blocks.push({ type: 'paragraph', content: parseInline(paragraph.join('\n')) })
  }
  if (blocks.length === 0) blocks.push({ type: 'paragraph', content: [] })
  return { type: 'doc', content: blocks }
}
