import type { Editor } from '@tiptap/core'
import type { Node as ProseNode, ResolvedPos, Schema } from '@tiptap/pm/model'
import { TextSelection } from '@tiptap/pm/state'

type LineSpan = {
  text: string
  contentFrom: number
  contentTo: number
  absFrom: number
  absTo: number
  blockIndex: number
  code: boolean
  language: string | null
}

function textblockDepth(pos: { depth: number; node: (depth: number) => { isTextblock: boolean } }): number | null {
  for (let depth = pos.depth; depth > 0; depth -= 1) {
    if (pos.node(depth).isTextblock) return depth
  }
  return null
}

function resolveTextblock(
  pos: ResolvedPos,
  side: 'from' | 'to',
): { node: ProseNode; pos: number; parent: ProseNode } | null {
  const depth = textblockDepth(pos)
  if (depth != null) {
    return { node: pos.node(depth), pos: pos.before(depth), parent: pos.node(depth - 1) }
  }
  const neighbor = side === 'from' ? pos.nodeAfter : pos.nodeBefore
  if (!neighbor?.isTextblock) return null
  const start = side === 'from' ? pos.pos : pos.pos - neighbor.nodeSize
  return { node: neighbor, pos: start, parent: pos.parent }
}

function languageOf(node: ProseNode): string | null {
  const language = node.attrs.language
  return typeof language === 'string' && language.length > 0 ? language : null
}

/** Lines inside one textblock. Hard breaks and newline characters are the boundaries. */
function linesOf(block: ProseNode, blockPos: number, blockIndex: number): LineSpan[] {
  const origin = blockPos + 1
  const code = block.type.name === 'codeBlock'
  const language = languageOf(block)
  const lines: LineSpan[] = []
  let lineStart = 0
  let text = ''
  const push = (contentTo: number) => {
    lines.push({
      text,
      contentFrom: lineStart,
      contentTo,
      absFrom: origin + lineStart,
      absTo: origin + contentTo,
      blockIndex,
      code,
      language,
    })
    text = ''
  }
  block.forEach((child, offset) => {
    if (child.type.name === 'hardBreak') {
      push(offset)
      lineStart = offset + child.nodeSize
      return
    }
    if (child.isText && child.text?.includes('\n')) {
      let cursor = 0
      const parts = child.text.split('\n')
      parts.forEach((part, index) => {
        if (index > 0) {
          push(offset + cursor - 1)
          lineStart = offset + cursor
        }
        text += part
        cursor += part.length
        if (index < parts.length - 1) cursor += 1
      })
      return
    }
    text += child.textContent
  })
  push(block.content.size)
  return lines
}

function lineOverlaps(line: LineSpan, from: number, to: number): boolean {
  if (from === to) return line.absFrom <= from && from <= line.absTo
  if (line.absFrom === line.absTo) return from <= line.absFrom && to > line.absFrom
  return from < line.absTo && to > line.absFrom
}

function keptNodes(block: ProseNode, lines: readonly LineSpan[], schema: Schema): ProseNode[] {
  if (lines.length === 0) return []
  if (block.type.name === 'codeBlock') {
    const codeBlock = schema.nodes.codeBlock
    if (!codeBlock) return []
    const text = lines.map((line) => line.text).join('\n')
    return [codeBlock.create({ language: languageOf(block) }, text.length > 0 ? schema.text(text) : null)]
  }
  const paragraph = schema.nodes.paragraph
  if (!paragraph) return []
  const fragment = block.content.cut(lines[0].contentFrom, lines[lines.length - 1].contentTo)
  return [paragraph.create(null, fragment)]
}

export function toggleInlineCode(editor: Editor): boolean {
  return editor.chain().focus().toggleCode().run()
}

/**
 * Code block follows the selected lines. A partial selection inside a multi-line
 * paragraph splits that paragraph, and the text outside the selection stays as it was.
 * A caret inside an existing code block turns the whole block back into paragraphs.
 */
export function toggleCodeBlock(editor: Editor): boolean {
  return editor.chain().focus().command(({ tr, state }) => {
    const { selection } = state
    const fromBlock = resolveTextblock(selection.$from, 'from')
    const toBlock = resolveTextblock(selection.$to, 'to') ?? fromBlock
    if (!fromBlock || !toBlock) return false
    const sameParent = fromBlock.parent === toBlock.parent
    let first = fromBlock
    let last = sameParent ? toBlock : fromBlock
    if (last.pos < first.pos) {
      const swap = first
      first = last
      last = swap
    }
    const rangeEnd = last.pos + last.node.nodeSize
    const blocks: { node: ProseNode; pos: number }[] = []
    state.doc.nodesBetween(first.pos, rangeEnd, (node, pos) => {
      if (!node.isTextblock) return
      if (pos < first.pos || pos >= rangeEnd) return false
      blocks.push({ node, pos })
      return false
    })
    if (blocks.length === 0) return false

    const lines = blocks.flatMap((block, index) => linesOf(block.node, block.pos, index))
    const collapsedInCode = selection.empty && blocks.length === 1 && blocks[0].node.type.name === 'codeBlock'
    const selected = collapsedInCode
      ? lines
      : lines.filter((line) => lineOverlaps(line, selection.from, selection.to))
    if (selected.length === 0) return false
    const firstLine = selected[0]
    const lastLine = selected[selected.length - 1]
    const startIndex = lines.indexOf(firstLine)
    const endIndex = lines.indexOf(lastLine)
    const chosen = lines.slice(startIndex, endIndex + 1)
    const prefix = lines.slice(0, startIndex).filter((line) => line.blockIndex === firstLine.blockIndex)
    const suffix = lines.slice(endIndex + 1).filter((line) => line.blockIndex === lastLine.blockIndex)

    const paragraph = state.schema.nodes.paragraph
    const codeBlock = state.schema.nodes.codeBlock
    if (!paragraph || !codeBlock) return false
    const toParagraphs = chosen.every((line) => line.code)
    const middle = toParagraphs
      ? chosen.map((line) => paragraph.create(null, line.text.length > 0 ? state.schema.text(line.text) : null))
      : [codeBlock.create(
        { language: chosen.find((line) => line.language)?.language ?? null },
        chosen.some((line) => line.text.length > 0) ? state.schema.text(chosen.map((line) => line.text).join('\n')) : null,
      )]
    const nodes = [
      ...keptNodes(blocks[firstLine.blockIndex].node, prefix, state.schema),
      ...middle,
      ...keptNodes(blocks[lastLine.blockIndex].node, suffix, state.schema),
    ]
    if (nodes.length === 0) return false
    const start = blocks[0].pos
    const end = blocks[blocks.length - 1].pos + blocks[blocks.length - 1].node.nodeSize
    try {
      tr.replaceWith(start, end, nodes)
    } catch {
      return false
    }
    const prefixSize = prefix.length > 0 ? nodes[0].nodeSize : 0
    const cursor = Math.min(start + prefixSize + 1, tr.doc.content.size)
    tr.setSelection(TextSelection.near(tr.doc.resolve(cursor)))
    return true
  }).run()
}
