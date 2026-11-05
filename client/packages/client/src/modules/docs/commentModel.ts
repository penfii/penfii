import type { RichTextDoc } from '@penfii/contracts'

export type PickedMention = { userId: string; name: string }

type Inline = { type: 'text'; text: string } | { type: 'mention'; attrs: { occurrenceId: string; kind: 'USER'; userId: string } }

/**
 * Turns what was typed into RichText v1. Each line is a paragraph; `@Name` that was picked from the
 * member list becomes a mention node, anything else stays text (a typed name that was never picked
 * mentions nobody).
 */
export function commentToRichText(text: string, mentions: readonly PickedMention[], newId: () => string = () => crypto.randomUUID()): RichTextDoc {
  const byLength = [...mentions].sort((a, b) => b.name.length - a.name.length)
  const paragraphs = text.split('\n').map((line) => {
    const content: Inline[] = []
    let rest = line
    while (rest.length > 0) {
      let best: { at: number; mention: PickedMention } | null = null
      for (const mention of byLength) {
        const at = rest.indexOf(`@${mention.name}`)
        if (at >= 0 && (best === null || at < best.at)) best = { at, mention }
      }
      if (!best) {
        content.push({ type: 'text', text: rest })
        break
      }
      if (best.at > 0) content.push({ type: 'text', text: rest.slice(0, best.at) })
      content.push({ type: 'mention', attrs: { occurrenceId: newId(), kind: 'USER', userId: best.mention.userId } })
      rest = rest.slice(best.at + 1 + best.mention.name.length)
    }
    return { type: 'paragraph' as const, content }
  })
  return { type: 'doc', content: paragraphs } as unknown as RichTextDoc
}
