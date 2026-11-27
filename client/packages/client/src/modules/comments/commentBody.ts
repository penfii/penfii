import type { RichTextDoc } from '@penfii/contracts'
import { isEditableRichText, isFormattedRichText, mentionedUserIds } from '../../richtext'

/** A document with one paragraph per line of `text`, which is how a plain comment has always been stored. */
export function paragraphsOf(text: string): RichTextDoc {
  const lines = text.replace(/\s+$/, '').split('\n')
  return {
    type: 'doc',
    content: lines.map((line) => ({ type: 'paragraph' as const, content: line ? [{ type: 'text' as const, text: line }] : [] })),
  }
}

/**
 * What the editor starts a comment from. A stored body the editor carries whole (plain paragraphs, headings, lists, quotes,
 * code, links, marks and the mentions it holds) is used as it is, so everything in it survives a change to any other part.
 * Any other body (one in another document format, or with a node the editor does not know) is edited as its plain text:
 * what a person sees in the comment is what they edit.
 */
export function editableBody(body: unknown, plainTextOf: (body: unknown) => string): RichTextDoc {
  return isEditableRichText(body) ? body : paragraphsOf(plainTextOf(body))
}

/** Whether a stored body is shown with the rich view (formatting, or a mention that needs a name) rather than as lines of text. */
export function showsRich(body: unknown): body is RichTextDoc {
  return isFormattedRichText(body) || (isEditableRichText(body) && mentionedUserIds(body).length > 0)
}

const isBlankBlock = (block: RichTextDoc['content'][number]): boolean =>
  block.type === 'paragraph' && (block.content ?? []).every((node) => node.type === 'text' && node.text.trim() === '')

/** The body without the empty paragraphs before its first line and after its last (the Enter pressed at the end of the box). */
export function withoutBlankEdges(doc: RichTextDoc): RichTextDoc {
  const blocks = [...(doc.content ?? [])]
  while (blocks.length > 0 && isBlankBlock(blocks[0]!)) blocks.shift()
  while (blocks.length > 0 && isBlankBlock(blocks[blocks.length - 1]!)) blocks.pop()
  return { ...doc, content: blocks }
}
