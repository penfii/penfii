import type { Message, RichTextDoc, Ticket } from '@penfii/contracts'
import { api } from '../../api'
import { newIdempotencyKey } from '@penfii/api-client'
import { plainText } from '../../richtext/text'
import type { DisplayNames } from '../../richtext'
import type { ChatSource } from './panelParam'

const TITLE_MAX = 120
const KEY = (messageId: string) => `penfii.ticketFromChat.${messageId}`

/** What the Create form starts with: text the message already holds, nothing interpreted. */
export type ChatPrefill = { title: string; description: string }

function readable(body: RichTextDoc | null, names: DisplayNames): string {
  if (!body) return ''
  return plainText(body).replace(/@([0-9a-f-]{36})/g, (match, id: string) => (names[id] ? `@${names[id]}` : match)).trim()
}

/** The first line of the message is the title; the whole text is the description when it says more than the title. */
export function prefillFromMessage(message: Message, names: DisplayNames): ChatPrefill {
  const text = readable(message.body, names)
  const first = text.split('\n').find((line) => line.trim())?.trim() ?? ''
  const title = first.length > TITLE_MAX ? `${first.slice(0, TITLE_MAX - 1)}…` : first
  return { title, description: text === title ? '' : text }
}

/** Kept for the tab, so reloading the page with the form open does not lose the starting text. */
export function rememberPrefill(messageId: string, prefill: ChatPrefill): void {
  try {
    sessionStorage.setItem(KEY(messageId), JSON.stringify(prefill))
  } catch {
    // The form simply starts empty.
  }
}

export function recallPrefill(messageId: string): ChatPrefill | null {
  try {
    const raw = sessionStorage.getItem(KEY(messageId))
    return raw ? (JSON.parse(raw) as ChatPrefill) : null
  } catch {
    return null
  }
}

export function forgetPrefill(messageId: string): void {
  try {
    sessionStorage.removeItem(KEY(messageId))
  } catch {
    // Nothing to forget.
  }
}

/**
 * Tells the thread the Ticket was made: an ordinary reply in the message's thread whose text links to the
 * Ticket. The message is the existing kind, so it is listed, read and kept like any other.
 */
export async function announceTicket(source: ChatSource, ticket: Ticket, workspaceId: string, text: string): Promise<void> {
  const href = `${window.location.origin}/w/${workspaceId}/projects/${ticket.projectId}/work?panel=ticket:${ticket.id}`
  const label = `${ticket.key} ${ticket.title.length > 80 ? `${ticket.title.slice(0, 79)}…` : ticket.title}`
  const body: RichTextDoc = {
    type: 'doc',
    content: [
      {
        type: 'paragraph',
        content: [
          { type: 'text', text: `${text} ` },
          { type: 'text', text: label, marks: [{ type: 'link', attrs: { href } }] },
        ],
      },
    ],
  }
  await api.sendMessage(source.conversationId, { bodySchemaVersion: 1, body, threadRootId: source.threadRootId, replyToId: null }, newIdempotencyKey())
  forgetPrefill(source.messageId)
}
