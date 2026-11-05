import type { RichTextDoc } from '@penfii/contracts'

export const EMPTY_RICH_TEXT: RichTextDoc = {
  type: 'doc',
  content: [{ type: 'paragraph', content: [] }],
}
