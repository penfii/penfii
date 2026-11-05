import { useQuery } from '@tanstack/react-query'
import type { WorkflowState } from '@penfii/contracts'
import { useI18n } from '../../i18n/I18nProvider'
import { Popover } from './Popover'
import type { WorkTicket } from './viewModel'
import { queries } from '../../queries'

/**
 * Changes a Ticket's State from a list or a table. The choices are the States of the Ticket's own
 * Project, so a wider scope never offers a State the Ticket cannot have. They load when the menu
 * opens (or come from `known` when the Project's States are already on the page).
 */
export function StateMenu({ ticket, known, disabled, onPick }: { ticket: WorkTicket; known?: WorkflowState[]; disabled: boolean; onPick: (stateId: string) => void }) {
  const { t } = useI18n()
  if (disabled) {
    return <span className={`state-chip state-${ticket.stateCategory.toLowerCase()}`}>{ticket.stateName}</span>
  }
  return (
    <Popover label={ticket.stateName} className={`state-chip state-${ticket.stateCategory.toLowerCase()} state-trigger`}>
      {(close) => (
        <StateChoices
          projectId={ticket.projectId}
          known={known}
          currentId={ticket.statusId}
          onPick={(id) => {
            close()
            onPick(id)
          }}
          title={t('work.state.change', { key: ticket.key })}
        />
      )}
    </Popover>
  )
}

function StateChoices({ projectId, known, currentId, title, onPick }: { projectId: string; known?: WorkflowState[]; currentId: string; title: string; onPick: (id: string) => void }) {
  const { t } = useI18n()
  const query = useQuery({ ...queries.projectWorkflow(projectId), enabled: !known, staleTime: 30_000 })
  const states = known ?? query.data?.states ?? []
  if (!known && query.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (!known && query.isError) return <p className="board-error" role="alert">{t('work.state.loadError')}</p>
  return (
    <div className="state-choices" role="group" aria-label={title}>
      {[...states]
        .sort((a, b) => a.position - b.position)
        .map((state) => (
          <button key={state.id} type="button" className={state.id === currentId ? 'state-choice on' : 'state-choice'} aria-pressed={state.id === currentId} disabled={state.id === currentId} onClick={() => onPick(state.id)}>
            <span className={`state-dot state-${state.category.toLowerCase()}`} aria-hidden="true" />
            {state.name}
          </button>
        ))}
    </div>
  )
}
