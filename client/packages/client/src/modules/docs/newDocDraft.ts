import type { DocContent } from '@penfii/contracts'

/** One unsaved new document per person and workspace. It never goes to the server. */
const PREFIX = 'penfii.doc.newDraft.v1'

export type NewDocDraft = {
  title: string
  content: DocContent
}

const EMPTY_BLOCKS: unknown[] = [{ type: 'paragraph', props: {}, content: [], children: [] }]

export function newDocDraftKey(userId: string, workspaceId: string): string {
  return `${PREFIX}:${userId}:${workspaceId}`
}

function storage(): Storage | null {
  try {
    return window.localStorage
  } catch {
    return null
  }
}

/** Drops image and file blocks. A page does not exist yet, so a file cannot be attached. */
export function draftContent(content: DocContent): DocContent {
  const blocks = content.blocks.filter((block) => {
    const type = typeof block === 'object' && block !== null ? (block as { type?: unknown }).type : undefined
    return type !== 'image' && type !== 'file' && type !== 'video' && type !== 'audio'
  })
  return { schema: 'penfii.doc', version: 1, blocks: blocks.length > 0 ? blocks : EMPTY_BLOCKS }
}

export function isBlankDraft(draft: NewDocDraft): boolean {
  if (draft.title.trim().length > 0) return false
  return draft.content.blocks.every((block) => {
    if (typeof block !== 'object' || block === null) return false
    const { type, content, children } = block as { type?: unknown; content?: unknown; children?: unknown }
    const noText = content === undefined || (Array.isArray(content) && content.every((item) => {
      const text = (item as { text?: unknown }).text
      return text === undefined || text === ''
    }))
    return type === 'paragraph' && noText && (children === undefined || (Array.isArray(children) && children.length === 0))
  })
}

export function readNewDocDraft(userId: string, workspaceId: string): NewDocDraft | null {
  const store = storage()
  if (!store) return null
  try {
    const raw = store.getItem(newDocDraftKey(userId, workspaceId))
    if (!raw) return null
    const parsed = JSON.parse(raw) as Partial<NewDocDraft>
    if (!parsed.content || parsed.content.schema !== 'penfii.doc' || !Array.isArray(parsed.content.blocks)) return null
    return { title: typeof parsed.title === 'string' ? parsed.title : '', content: draftContent(parsed.content) }
  } catch {
    return null
  }
}

/** Writes the one draft, or removes it when there is nothing to keep. Returns whether storage accepted the write. */
export function writeNewDocDraft(userId: string, workspaceId: string, draft: NewDocDraft): boolean {
  const store = storage()
  if (!store) return false
  const key = newDocDraftKey(userId, workspaceId)
  try {
    const kept = { title: draft.title, content: draftContent(draft.content) }
    if (isBlankDraft(kept)) {
      store.removeItem(key)
      return true
    }
    store.setItem(key, JSON.stringify(kept))
    return true
  } catch {
    return false
  }
}

export function clearNewDocDraft(userId: string, workspaceId: string): void {
  try {
    storage()?.removeItem(newDocDraftKey(userId, workspaceId))
  } catch {
    // A private window can refuse storage. Leaving the key is the same as having none.
  }
}
