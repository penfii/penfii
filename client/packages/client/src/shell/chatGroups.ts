/** A person's own chat sections. Stored in this browser; they do not change membership or permissions. */

export type ChatSection = {
  id: string
  name: string
  conversationIds: string[]
}

export type ChatSections = { sections: ChatSection[] }

const MAX_NAME = 40
const MAX_SECTIONS = 20
export const CHANNEL_PREVIEW = 10

export function sectionsKey(workspaceId: string): string {
  return `penfii.chat.sections.${workspaceId}`
}

export function emptySections(): ChatSections {
  return { sections: [] }
}

export function parseSections(raw: string | null): ChatSections {
  if (!raw) return emptySections()
  try {
    const parsed: unknown = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object' || !Array.isArray((parsed as { sections?: unknown }).sections)) return emptySections()
    const sections: ChatSection[] = []
    for (const item of (parsed as { sections: unknown[] }).sections) {
      if (!item || typeof item !== 'object') continue
      const row = item as { id?: unknown; name?: unknown; conversationIds?: unknown }
      if (typeof row.id !== 'string' || typeof row.name !== 'string' || !Array.isArray(row.conversationIds)) continue
      const name = row.name.trim().slice(0, MAX_NAME)
      if (!name) continue
      const conversationIds = row.conversationIds.filter((id): id is string => typeof id === 'string' && id.length > 0)
      sections.push({ id: row.id, name, conversationIds: [...new Set(conversationIds)] })
      if (sections.length >= MAX_SECTIONS) break
    }
    return claimOnce({ sections })
  } catch {
    return emptySections()
  }
}

/** One conversation lives in one section. The first section in order keeps it. */
export function claimOnce(layout: ChatSections): ChatSections {
  const seen = new Set<string>()
  return {
    sections: layout.sections.map((section) => ({
      ...section,
      conversationIds: section.conversationIds.filter((id) => {
        if (seen.has(id)) return false
        seen.add(id)
        return true
      }),
    })),
  }
}

export function loadSections(workspaceId: string): ChatSections {
  try {
    return parseSections(localStorage.getItem(sectionsKey(workspaceId)))
  } catch {
    return emptySections()
  }
}

export function saveSections(workspaceId: string, layout: ChatSections): void {
  try {
    localStorage.setItem(sectionsKey(workspaceId), JSON.stringify(claimOnce(layout)))
  } catch {
    // A private arrangement. The screen still shows the change until reload.
  }
}

export function placedIds(layout: ChatSections): Set<string> {
  return new Set(layout.sections.flatMap((section) => section.conversationIds))
}

export function createSection(layout: ChatSections, name: string): ChatSections {
  const trimmed = name.trim().slice(0, MAX_NAME)
  if (!trimmed || layout.sections.length >= MAX_SECTIONS) return layout
  return { sections: [...layout.sections, { id: crypto.randomUUID(), name: trimmed, conversationIds: [] }] }
}

export function renameSection(layout: ChatSections, id: string, name: string): ChatSections {
  const trimmed = name.trim().slice(0, MAX_NAME)
  if (!trimmed) return layout
  return { sections: layout.sections.map((section) => (section.id === id ? { ...section, name: trimmed } : section)) }
}

/** Removing a section puts its conversations back in the built-in lists. It does not delete them. */
export function deleteSection(layout: ChatSections, id: string): ChatSections {
  return { sections: layout.sections.filter((section) => section.id !== id) }
}

function without(layout: ChatSections, conversationId: string): ChatSections {
  return {
    sections: layout.sections.map((section) => ({
      ...section,
      conversationIds: section.conversationIds.filter((id) => id !== conversationId),
    })),
  }
}

/** `sectionId` null returns the conversation to Channels, Project channels, or Direct messages. */
export function moveConversation(layout: ChatSections, conversationId: string, sectionId: string | null, index?: number): ChatSections {
  const cleared = without(layout, conversationId)
  if (!sectionId) return cleared
  return {
    sections: cleared.sections.map((section) => {
      if (section.id !== sectionId) return section
      const ids = [...section.conversationIds]
      const at = index == null ? ids.length : Math.max(0, Math.min(index, ids.length))
      ids.splice(at, 0, conversationId)
      return { ...section, conversationIds: ids }
    }),
  }
}

export function reorderSections(layout: ChatSections, fromId: string, toId: string): ChatSections {
  const list = [...layout.sections]
  const from = list.findIndex((section) => section.id === fromId)
  const to = list.findIndex((section) => section.id === toId)
  if (from < 0 || to < 0 || from === to) return layout
  const [item] = list.splice(from, 1)
  if (!item) return layout
  list.splice(to, 0, item)
  return { sections: list }
}

export function pruneSections(layout: ChatSections, live: ReadonlySet<string>): ChatSections {
  return {
    sections: layout.sections.map((section) => ({
      ...section,
      conversationIds: section.conversationIds.filter((id) => live.has(id)),
    })),
  }
}

/**
 * Unread rows stay at the top. A long list shows the first `limit` rows, and every unread row even past that,
 * until the person asks for the rest.
 */
export function visibleSlice<T>(items: readonly T[], expanded: boolean, unread: (item: T) => boolean, limit = CHANNEL_PREVIEW): { shown: T[]; more: number } {
  const hot = items.filter((item) => unread(item))
  const rest = items.filter((item) => !unread(item))
  const ordered = [...hot, ...rest]
  if (expanded || ordered.length <= limit) return { shown: ordered, more: 0 }
  const keep = Math.max(limit, hot.length)
  return { shown: ordered.slice(0, keep), more: ordered.length - keep }
}
