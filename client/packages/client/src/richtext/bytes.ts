import type { RichTextDoc } from '@penfii/contracts'

export function messageBodyBytes(doc: RichTextDoc): number {
  return new TextEncoder().encode(JSON.stringify(doc)).length
}
