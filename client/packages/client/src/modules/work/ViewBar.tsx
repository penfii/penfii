import { useState, type FormEvent, type ReactNode } from 'react'
import type { Channel, ProjectMember, ProjectPlanning, ProjectView, ScopeProject, TableColumn, TicketLabel, TicketType, ViewDefinition, ViewLayout, ViewTargetKind } from '@penfii/contracts'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { FilterPanel } from './FilterPanel'
import { Popover } from './Popover'
import { useIsMobile } from '../../design/useIsMobile'
import { SORT_FIELDS, TABLE_COLUMNS, DEFAULT_TABLE_COLUMNS, filterCount, groupsFor, layoutsFor } from './viewModel'

export type ViewSaveError = 'nameTaken' | 'conflict' | 'forbidden' | 'generic' | null

/**
 * The View controls above the Board: pick a saved View, then narrow, group and sort it. Changes
 * stay a draft in the URL until they are saved, so a shared View is never altered by browsing.
 */
export function ViewBar({
  target,
  layout,
  isAdmin,
  views,
  activeId,
  definition,
  dirty,
  labels,
  members,
  types,
  planning,
  channels,
  projects,
  onLayout,
  onSelect,
  onChange,
  onReset,
  onSave,
  onSaveAs,
  onRename,
  onDelete,
  actions,
}: {
  /** Where the person is looking from. A Project offers a Board; a Team or the Workspace do not. */
  target: ViewTargetKind
  layout: ViewLayout
  isAdmin: boolean
  views: ProjectView[]
  activeId: string | null
  definition: ViewDefinition
  dirty: boolean
  labels: TicketLabel[]
  members: ProjectMember[]
  types: TicketType[]
  planning: ProjectPlanning | undefined
  channels?: Channel[]
  /** The Projects of a Team or Workspace scope; absent in a Project. */
  projects?: ScopeProject[]
  onLayout: (layout: ViewLayout) => void
  onSelect: (id: string | null) => void
  onChange: (definition: ViewDefinition) => void
  onReset: () => void
  onSave: () => Promise<ViewSaveError>
  onSaveAs: (name: string, scope: 'PERSONAL' | 'PROJECT') => Promise<ViewSaveError>
  onRename: (name: string) => Promise<ViewSaveError>
  onDelete: () => Promise<ViewSaveError>
  /** Board-level controls that sit at the end of the same row (settings). */
  actions?: ReactNode
}) {
  const { t } = useI18n()
  const mobile = useIsMobile()
  const active = views.find((view) => view.id === activeId)
  const count = filterCount(definition.filter)
  const groups = groupsFor(target, layout)
  const [error, setError] = useState<ViewSaveError>(null)
  const errorKey = error ? (`view.error.${error}` as const) : null

  async function run(action: () => Promise<ViewSaveError>): Promise<boolean> {
    const result = await action()
    setError(result)
    return result === null
  }

  return (
    <div className="view-bar">
      <div className="view-tabs" role="group" aria-label={t('view.bar.label')}>
        <button type="button" className={activeId === null ? 'view-tab on' : 'view-tab'} aria-pressed={activeId === null} onClick={() => onSelect(null)}>
          {t('view.default')}
        </button>
        {views.map((view) => (
          <button key={view.id} type="button" className={view.id === activeId ? 'view-tab on' : 'view-tab'} aria-pressed={view.id === activeId} onClick={() => onSelect(view.id)} title={view.scope === 'PROJECT' ? t('view.shared') : undefined}>
            {view.name}
            {view.scope === 'PROJECT' ? <small className="view-tab-mark">{t('view.shared')}</small> : null}
          </button>
        ))}
        {active?.canEdit ? (
          <Popover label="⋯" wide>
            {(close) => (
              <ManageView
                key={active.id}
                name={active.name}
                onRename={(name) => run(() => onRename(name)).then((ok) => ok && close())}
                onDelete={() => run(onDelete).then((ok) => ok && close())}
              />
            )}
          </Popover>
        ) : null}
      </div>

      <div className="view-layouts" role="group" aria-label={t('view.layout')}>
        {layoutsFor(target).map((item) => (
          <button key={item} type="button" data-layout={item} className={item === layout ? 'view-layout on' : 'view-layout'} aria-pressed={item === layout} onClick={() => onLayout(item)}>
            {t(`view.layout.${item}`)}
          </button>
        ))}
      </div>

      <div className="view-controls">
        <input
          type="search"
          className="view-search"
          aria-label={t('view.filter.text')}
          placeholder={t('view.filter.text')}
          value={definition.filter.text ?? ''}
          onChange={(event) => onChange({ ...definition, filter: { ...definition.filter, text: event.target.value || null } })}
        />
        <Popover label={t('view.filter')} badge={count > 0 ? <span className="pop-count">{count}</span> : null}>
          {() => <FilterPanel filter={definition.filter} labels={labels} members={members} types={types} planning={planning} channels={channels} projects={projects} onChange={(filter) => onChange({ ...definition, filter })} />}
        </Popover>
        {mobile ? (
          <Popover label={t('view.sortGroup')} className="btn btn-secondary pop-trigger">
            {() => (
              <div className="filter-panel">
                <div className="filter-panel-head"><strong>{t('view.sortGroup')}</strong></div>
                <SortGroupControls definition={definition} groups={groups} onChange={onChange} />
              </div>
            )}
          </Popover>
        ) : (
          <div className="view-inline-order">
            <SortGroupControls definition={definition} groups={groups} onChange={onChange} />
          </div>
        )}
        <Popover label={t('view.display')} align="end">
          {() => <DisplayPanel definition={definition} layout={layout} wide={target !== 'PROJECT'} onChange={onChange} />}
        </Popover>
        {actions}
      </div>

      {dirty || error ? (
        <div className="view-dirty" role="status">
          {dirty ? <span>{t('view.dirty')}</span> : null}
          {error && errorKey ? <span className="view-error" role="alert">{t(errorKey)}</span> : null}
          {dirty ? (
            <span className="view-dirty-actions">
              {active?.canEdit ? <Button type="button" onClick={() => void run(onSave)}>{t('view.save')}</Button> : null}
              <Popover label={active ? t('view.saveAs') : t('view.new')} align="end" wide>
                {(close) => <SaveAs canShare={isAdmin && target === 'PROJECT'} onSave={(name, scope) => run(() => onSaveAs(name, scope)).then((ok) => ok && close())} />}
              </Popover>
              <Button type="button" variant="ghost" onClick={() => { setError(null); onReset() }}>{t('view.reset')}</Button>
            </span>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

/** Group, sort field and direction. The wide toolbar shows them inline and a phone inside one sheet; the controls are the same. */
function SortGroupControls({ definition, groups, onChange }: { definition: ViewDefinition; groups: readonly ViewDefinition['group'][]; onChange: (definition: ViewDefinition) => void }) {
  const { t } = useI18n()
  const { sort } = definition
  return (
    <>
      <label className="view-select">
        <span>{t('view.group')}</span>
        <select value={definition.group} onChange={(event) => onChange({ ...definition, group: event.target.value as ViewDefinition['group'] })}>
          {groups.map((group) => (
            <option key={group} value={group}>{t(`view.group.${group}`)}</option>
          ))}
        </select>
      </label>
      <label className="view-select">
        <span>{t('view.sort')}</span>
        <select value={sort.field} onChange={(event) => onChange({ ...definition, sort: { ...sort, field: event.target.value as ViewDefinition['sort']['field'] } })}>
          {SORT_FIELDS.map((field) => (
            <option key={field} value={field}>{t(`view.sort.${field}`)}</option>
          ))}
        </select>
      </label>
      <button
        type="button"
        className="btn btn-secondary view-dir"
        aria-label={t(`view.sort.${sort.dir}`)}
        title={t(`view.sort.${sort.dir}`)}
        onClick={() => onChange({ ...definition, sort: { ...sort, dir: sort.dir === 'ASC' ? 'DESC' : 'ASC' } })}
      >
        {sort.dir === 'ASC' ? '↑' : '↓'}
      </button>
    </>
  )
}

/** Presentation only: nothing here changes which Tickets a View shows. */
function DisplayPanel({ definition, layout, wide, onChange }: { definition: ViewDefinition; layout: ViewLayout; wide: boolean; onChange: (definition: ViewDefinition) => void }) {
  const { t } = useI18n()
  const mobile = useIsMobile()
  const display = definition.display
  const chosen = new Set<TableColumn>(display.columns.length > 0 ? display.columns : DEFAULT_TABLE_COLUMNS)
  const set = (patch: Partial<ViewDefinition['display']>) => onChange({ ...definition, display: { ...display, ...patch } })
  function toggleColumn(column: TableColumn) {
    const next = new Set(chosen)
    if (next.has(column)) next.delete(column)
    else next.add(column)
    // An empty list means "the default set", so keep at least the Key and Title.
    next.add('KEY')
    next.add('TITLE')
    const list = TABLE_COLUMNS.filter((item) => next.has(item))
    const isDefault = list.length === DEFAULT_TABLE_COLUMNS.length && list.every((item) => DEFAULT_TABLE_COLUMNS.includes(item))
    set({ columns: isDefault ? [] : list })
  }
  return (
    <div className="filter-panel">
      <div className="filter-panel-head">
        <strong>{t('view.display')}</strong>
      </div>
      <label className="filter-choice">
        <input type="checkbox" checked={display.subtasks} onChange={(event) => set({ subtasks: event.target.checked })} />
        <span>{t('view.display.subtasks')}</span>
      </label>
      <label className="filter-choice">
        <input type="checkbox" checked={display.compact} onChange={(event) => set({ compact: event.target.checked })} />
        <span>{t('view.display.compact')}</span>
      </label>
      {layout === 'TABLE' || (layout === 'LIST' && mobile) ? (
        <fieldset className="filter-group">
          <legend>{mobile ? t('view.display.fields') : t('view.display.columns')}</legend>
          {TABLE_COLUMNS.map((column) => (
            <label key={column} className="filter-choice">
              <input type="checkbox" checked={chosen.has(column) || (wide && column === 'PROJECT')} disabled={column === 'KEY' || column === 'TITLE' || (wide && column === 'PROJECT')} onChange={() => toggleColumn(column)} />
              <span>{t(`view.column.${column}`)}</span>
            </label>
          ))}
        </fieldset>
      ) : null}
    </div>
  )
}

function SaveAs({ canShare, onSave }: { canShare: boolean; onSave: (name: string, scope: 'PERSONAL' | 'PROJECT') => Promise<unknown> }) {
  const { t } = useI18n()
  const [name, setName] = useState('')
  const [scope, setScope] = useState<'PERSONAL' | 'PROJECT'>('PERSONAL')
  const [busy, setBusy] = useState(false)
  async function submit(event: FormEvent) {
    event.preventDefault()
    if (!name.trim() || busy) return
    setBusy(true)
    try {
      await onSave(name.trim(), scope)
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="view-form" onSubmit={(event) => void submit(event)}>
      <label className="field">
        <span>{t('view.name')}</span>
        <input autoFocus value={name} maxLength={60} onChange={(event) => setName(event.target.value)} />
      </label>
      {canShare ? (
        <label className="field">
          <span>{t('view.scope')}</span>
          <select value={scope} onChange={(event) => setScope(event.target.value as 'PERSONAL' | 'PROJECT')}>
            <option value="PERSONAL">{t('view.scope.PERSONAL')}</option>
            <option value="PROJECT">{t('view.scope.PROJECT')}</option>
          </select>
          <small className="field-hint">{t('view.scope.help')}</small>
        </label>
      ) : null}
      <Button type="submit" disabled={!name.trim() || busy}>{t('view.create')}</Button>
    </form>
  )
}

function ManageView({ name, onRename, onDelete }: { name: string; onRename: (name: string) => Promise<unknown>; onDelete: () => Promise<unknown> }) {
  const { t } = useI18n()
  const [draft, setDraft] = useState(name)
  const [confirming, setConfirming] = useState(false)
  return (
    <div className="view-form">
      <form
        className="view-form"
        onSubmit={(event) => {
          event.preventDefault()
          if (draft.trim() && draft.trim() !== name) void onRename(draft.trim())
        }}
      >
        <label className="field">
          <span>{t('view.rename')}</span>
          <input value={draft} maxLength={60} onChange={(event) => setDraft(event.target.value)} />
        </label>
        <Button type="submit" variant="secondary" disabled={!draft.trim() || draft.trim() === name}>{t('view.rename')}</Button>
      </form>
      {confirming ? (
        <div className="view-confirm">
          <p>{t('view.delete.confirm', { name })}</p>
          <div className="dialog-actions">
            <Button type="button" variant="secondary" onClick={() => setConfirming(false)}>{t('view.cancel')}</Button>
            <Button type="button" onClick={() => void onDelete()}>{t('view.delete')}</Button>
          </div>
        </div>
      ) : (
        <Button type="button" variant="ghost" onClick={() => setConfirming(true)}>{t('view.delete')}</Button>
      )}
    </div>
  )
}
