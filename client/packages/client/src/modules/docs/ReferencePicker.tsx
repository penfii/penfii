import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { ReferenceKind } from '@penfii/contracts'
import { api } from '../../api'
import { Dialog } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'

type Option = { id: string; title: string; context: string }

/**
 * Chooses the target of a reference. The lists come from PostgreSQL endpoints the person already
 * uses (Tickets of the Workspace scope, the Doc tree), never from search.
 */
export function ReferencePicker({
  kind,
  workspaceId,
  projectId,
  onPick,
  onClose,
}: {
  kind: Exclude<ReferenceKind, 'USER' | 'RECORD'>
  workspaceId: string
  projectId: string | null
  onPick: (id: string) => void
  onClose: () => void
}) {
  const { t } = useI18n()
  const [text, setText] = useState('')
  const options = useQuery({
    queryKey: queryKeys.referencePicker(kind, workspaceId),
    queryFn: async (): Promise<Option[]> => {
      if (kind === 'TICKET') {
        const scope = await api.getScopeDataset(workspaceId, { kind: 'WORKSPACE' })
        return scope.tickets.map((ticket) => ({ id: ticket.id, title: `${ticket.key} ${ticket.title}`, context: ticket.projectName }))
      }
      if (kind === 'DOC') {
        const nodes = await api.getDocTree(workspaceId)
        return nodes.map((node) => ({ id: node.id, title: node.title || t('docs.untitled'), context: '' }))
      }
      return []
    },
  })
  const shown = useMemo(() => {
    const needle = text.trim().toLowerCase()
    const all = options.data ?? []
    const matching = needle ? all.filter((o) => o.title.toLowerCase().includes(needle) || o.context.toLowerCase().includes(needle)) : all
    return matching.slice(0, 50)
  }, [options.data, text])
  void projectId
  const title = kind === 'TICKET' ? t('docs.picker.ticket') : t('docs.picker.doc')
  return (
    <Dialog open title={title} onClose={onClose}>
      <div className="doc-picker">
        <input
          autoFocus
          type="search"
          aria-label={title}
          placeholder={t('docs.picker.search')}
          value={text}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.nativeEvent.isComposing && shown[0]) onPick(shown[0].id)
          }}
        />
        {options.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
        {options.isError ? <p className="board-error" role="alert">{t('docs.picker.error')}</p> : null}
        {options.isSuccess && shown.length === 0 ? <p className="hint">{t('docs.picker.empty')}</p> : null}
        <ul className="doc-picker-list">
          {shown.map((option) => (
            <li key={option.id}>
              <button type="button" onClick={() => onPick(option.id)}>
                <span>{option.title}</span>
                {option.context ? <small>{option.context}</small> : null}
              </button>
            </li>
          ))}
        </ul>
      </div>
    </Dialog>
  )
}
