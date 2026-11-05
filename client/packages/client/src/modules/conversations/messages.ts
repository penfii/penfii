import type { Message } from '@penfii/contracts'
import { api } from '../../api'

export function presentMessage(message: Message): Message {
  if (!message.deleted || message.body == null) return message
  return { ...message, body: null }
}

export function sortBySeq(messages: readonly Message[]): Message[] {
  return [...messages].sort((left, right) => left.streamSeq - right.streamSeq)
}

export function dedupeMessages(messages: readonly Message[]): Message[] {
  const map = new Map<string, Message>()
  for (const message of messages) map.set(message.id, presentMessage(message))
  return sortBySeq([...map.values()])
}

export function mergeMessages(current: readonly Message[], incoming: readonly Message[]): Message[] {
  return dedupeMessages([...current, ...incoming])
}

export async function loadThreadPages(rootId: string): Promise<{ generation: number | null; messages: Message[] }> {
  const all: Message[] = []
  let afterSeq: number | undefined
  let generation: number | null = null
  for (let pageIndex = 0; pageIndex < 10; pageIndex += 1) {
    const page = await api.listThread(rootId, { afterSeq, limit: 50 })
    if (page.generation != null) generation = page.generation
    const messages = page.messages.map(presentMessage)
    all.push(...messages)
    const last = messages[messages.length - 1]
    if (!last || messages.length < 50) break
    afterSeq = last.streamSeq
  }
  return { generation, messages: dedupeMessages(all) }
}
