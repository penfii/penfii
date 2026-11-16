import { filterSuggestionItems } from '@blocknote/core'
import { getDefaultReactSlashMenuItems, type DefaultReactSuggestionItem } from '@blocknote/react'
import { api } from '../../api'
import type { useI18n } from '../../i18n/I18nProvider'
import type { DocReferenceKind, docSchema } from './schema'

type DocBlockNoteEditor = typeof docSchema.BlockNoteEditor
type Translate = ReturnType<typeof useI18n>['t']

const FILE_ITEMS = new Set(['image', 'video', 'audio', 'file'])

/**
 * The `/` menu of an editor on the Docs model: BlockNote's blocks (file blocks only where the surface can
 * hold files, never emoji) and the two references, whose target `pickReference` asks the person for.
 */
export function slashMenuItems(
  editor: DocBlockNoteEditor,
  query: string,
  { files, t, pickReference }: { files: boolean; t: Translate; pickReference: (kind: DocReferenceKind, insert: (id: string) => void) => void },
): DefaultReactSuggestionItem[] {
  const base = getDefaultReactSlashMenuItems(editor).filter((item) => {
    const key = (item as { key?: string }).key
    if (key === 'emoji') return false
    return files || !key || !FILE_ITEMS.has(key)
  })
  const reference = (kind: DocReferenceKind, title: string, aliases: string[]): DefaultReactSuggestionItem => ({
    title,
    aliases,
    group: t('docs.menu.references'),
    onItemClick: () => pickReference(kind, (id) => {
      editor.focus()
      editor.insertInlineContent([{ type: 'reference', props: { kind, id } }, ' '])
    }),
  })
  return filterSuggestionItems(
    [
      ...base,
      reference('TICKET', t('docs.menu.ticket'), ['ticket', 'task', '티켓', '작업']),
      reference('DOC', t('docs.menu.doc'), ['doc', 'page', '문서', '페이지']),
    ],
    query,
  )
}

/** The `@` menu: people of the Workspace from the directory search. */
export async function mentionMenuItems(editor: DocBlockNoteEditor, workspaceId: string, query: string): Promise<DefaultReactSuggestionItem[]> {
  const found = await api.searchDirectory(workspaceId, { q: query, types: 'person', limit: 8 })
  return found.results.flatMap((hit) => {
    const person = hit.type === 'person' ? hit.person : undefined
    if (!person) return []
    const item: DefaultReactSuggestionItem = {
      title: person.displayName,
      subtext: person.organizationName,
      onItemClick: () => editor.insertInlineContent([{ type: 'mention', props: { userId: person.id } }, ' ']),
    }
    return [item]
  })
}
