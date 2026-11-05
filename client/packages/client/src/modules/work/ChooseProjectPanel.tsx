import { useState } from 'react'
import { useParams } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { ResizeHandle, type PaneControls } from '../../shell/ResizeHandle'
import { CloseIcon } from '../../design/icons'

/** A Ticket made from a Chat that belongs to no Project: the person chooses which Project it goes to. */
export function ChooseProjectPanel({ onChoose, onClose, pane }: { onChoose: (projectId: string) => void; onClose: () => void; pane?: PaneControls }) {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  const projects = (space.data?.projects ?? []).filter((project) => project.participating !== false)
  const [chosen, setChosen] = useState('')
  return (
    <aside className="detail-panel ticket-panel create-panel" aria-label={t('work.create.title')} onKeyDown={(event) => {
      if (event.key === 'Escape' && !event.defaultPrevented) onClose()
    }}>
      {pane ? <ResizeHandle pane={pane} label={t('work.create.resize')} resetLabel={t('layout.resetWidth')} /> : null}
      <div className="ticket-panel-head">
        <div className="head-cluster">
          <strong className="create-heading">{t('work.create.title')}</strong>
        </div>
        <button type="button" className="icon-button panel-close" aria-label={t('work.panel.close')} onClick={onClose}><CloseIcon /></button>
      </div>
      <div className="create-body">
        {space.isPending ? <p className="hint" role="status">{t('work.panel.loading')}</p> : null}
        {space.isSuccess && projects.length === 0 ? <p className="board-note" role="note">{t('work.create.pickProjectNone')}</p> : null}
        {projects.length > 0 ? (
          <form
            className="create-form"
            onSubmit={(event) => {
              event.preventDefault()
              if (chosen) onChoose(chosen)
            }}
          >
            <div className="create-fields">
            <label className="field">
              <span>{t('work.create.pickProject')}</span>
              <select value={chosen} onChange={(event) => setChosen(event.target.value)} autoFocus>
                <option value="">—</option>
                {projects.map((project) => (
                  <option key={project.id} value={project.id}>{project.name}</option>
                ))}
              </select>
            </label>
            </div>
            <div className="create-actions">
              <Button type="submit" disabled={!chosen}>{t('work.create.pickProjectNext')}</Button>
            </div>
          </form>
        ) : null}
      </div>
    </aside>
  )
}
