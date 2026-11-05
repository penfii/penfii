import { useRef, useState, type DragEvent } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import type { Department, Team } from '@penfii/contracts'
import { isApiError } from '@penfii/api-client'
import { api } from '../../api'
import { Badge, Button, Dialog, Field, Select } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { applyMoves, stageMove, subtreeIds, type OrgMove } from './orgTree'

type Drag = { kind: 'team' | 'department'; id: string }

/** Why a move was refused, in words for the person editing. */
function failureReason(caught: unknown, t: ReturnType<typeof useI18n>['t']): string {
  if (isApiError(caught)) {
    if (caught.code === 'department_cycle') return t('orgEdit.reason.cycle')
    if (caught.code === 'department_too_deep') return t('orgEdit.reason.deep')
    if (caught.code === 'team_archived') return t('orgEdit.reason.archived')
  }
  return t('orgEdit.reason.generic')
}

/**
 * Edits the department tree in two steps: moves are staged here, shown where they would land, and written only when applied.
 * Adding, renaming and deleting a department are single actions and are written at once. Departments grant no access.
 */
export function OrgStructureEditor({
  workspaceId,
  departments,
  teams,
  moves,
  onMoves,
}: {
  workspaceId: string
  departments: Department[]
  /** Teams that can be placed (not archived). */
  teams: Team[]
  moves: OrgMove[]
  onMoves: (moves: OrgMove[]) => void
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const effective = applyMoves(departments, teams, moves)
  const [drag, setDrag] = useState<Drag | null>(null)
  const [name, setName] = useState('')
  const [parentId, setParentId] = useState('')
  const [settings, setSettings] = useState<Department | null>(null)
  const [busy, setBusy] = useState(false)
  const [messages, setMessages] = useState<string[]>([])
  const [applied, setApplied] = useState(false)
  const nameInput = useRef<HTMLInputElement>(null)
  const nameOf = (id: string | null, kind: OrgMove['kind']) => departments.find((department) => department.id === id)?.name ?? (kind === 'team' ? t('orgEdit.noDepartment') : t('orgEdit.top'))
  const moved = (kind: OrgMove['kind'], id: string) => moves.some((move) => move.kind === kind && move.id === id)

  async function refresh() {
    await queryClient.invalidateQueries({ queryKey: queryKeys.departments(workspaceId) })
    await queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })
    await queryClient.invalidateQueries({ queryKey: queryKeys.all.adminAudit(workspaceId) })
  }
  function stage(move: OrgMove) {
    setApplied(false)
    setMessages([])
    onMoves(stageMove(moves, move))
  }
  function moveTeam(team: Team, to: string | null) {
    const from = team.departmentId ?? null
    if (from === to) return
    stage({ kind: 'team', id: team.id, name: team.name, from, to })
  }
  function moveDepartment(department: Department, to: string | null) {
    if (department.parentId === to) return
    // A department cannot go inside itself or below its own sub-departments.
    if (to !== null && subtreeIds(effective.departments, department.id).has(to)) {
      setMessages([t('orgEdit.cycle')])
      return
    }
    stage({ kind: 'department', id: department.id, name: department.name, from: department.parentId, to })
  }
  function drop(event: DragEvent, target: { department: string | null }) {
    event.preventDefault()
    const dragged = drag
    setDrag(null)
    if (!dragged) return
    if (dragged.kind === 'team') {
      const team = effective.teams.find((item) => item.id === dragged.id)
      if (team) moveTeam(team, target.department)
    } else {
      const department = effective.departments.find((item) => item.id === dragged.id)
      if (department) moveDepartment(department, target.department)
    }
  }
  const allow = (event: DragEvent, accepts: boolean) => {
    if (drag && accepts) event.preventDefault()
  }

  async function apply() {
    setBusy(true)
    setMessages([])
    const failed: string[] = []
    const left: OrgMove[] = []
    // Departments first, then Teams, each in the order they were staged.
    for (const move of [...moves.filter((item) => item.kind === 'department'), ...moves.filter((item) => item.kind === 'team')]) {
      try {
        if (move.kind === 'department') {
          const current = departments.find((department) => department.id === move.id)
          if (current) await api.updateDepartment(workspaceId, move.id, { name: current.name, parentId: move.to })
        } else {
          await api.placeTeam(workspaceId, move.id, move.to)
        }
      } catch (caught) {
        failed.push(t('orgEdit.failed', { name: move.name, reason: failureReason(caught, t) }))
        left.push(move)
      }
    }
    await refresh()
    onMoves(left)
    setMessages(failed)
    setApplied(failed.length === 0)
    setBusy(false)
  }

  function renderDepartment(department: Department) {
    const children = effective.departments.filter((item) => item.parentId === department.id)
    const here = effective.teams.filter((team) => team.departmentId === department.id)
    const blocked = subtreeIds(effective.departments, department.id)
    return (
      <section
        key={department.id}
        className={drag ? 'org-edit-dept is-target' : 'org-edit-dept'}
        draggable
        onDragStart={(event) => { event.stopPropagation(); setDrag({ kind: 'department', id: department.id }) }}
        onDragEnd={() => setDrag(null)}
        onDragOver={(event) => allow(event, !(drag?.kind === 'department' && blocked.has(drag.id)))}
        onDrop={(event) => { event.stopPropagation(); drop(event, { department: department.id }) }}
      >
        <header className="org-edit-head">
          <h2>{department.name}</h2>
          {moved('department', department.id) ? <Badge tone="accent">{t('orgEdit.moving')}</Badge> : null}
          <Select
            aria-label={t('orgEdit.moveDepartment', { name: department.name })}
            value={department.parentId ?? ''}
            onChange={(event) => moveDepartment(department, event.target.value || null)}
          >
            <option value="">{t('orgEdit.top')}</option>
            {effective.departments.filter((item) => !blocked.has(item.id)).map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
          </Select>
          <Button type="button" variant="secondary" onClick={() => { setParentId(department.id); nameInput.current?.focus() }}>{t('orgEdit.addChild')}</Button>
          <Button type="button" variant="secondary" onClick={() => setSettings(departments.find((item) => item.id === department.id) ?? department)}>{t('orgEdit.settings')}</Button>
        </header>
        {here.length === 0 && children.length === 0 ? <p className="hint">{t('orgEdit.empty')}</p> : null}
        <ul className="plain-list">
          {here.map((team) => (
            <li
              key={team.id}
              className="org-edit-team"
              draggable
              onDragStart={(event) => { event.stopPropagation(); setDrag({ kind: 'team', id: team.id }) }}
              onDragEnd={() => setDrag(null)}
            >
              <strong>{team.name}</strong>
              {moved('team', team.id) ? <Badge tone="accent">{t('orgEdit.moving')}</Badge> : null}
              <TeamPlace team={team} departments={effective.departments} onMove={(to) => moveTeam(team, to)} />
            </li>
          ))}
        </ul>
        {children.map(renderDepartment)}
      </section>
    )
  }

  const roots = effective.departments.filter((department) => department.parentId === null)
  const unplaced = effective.teams.filter((team) => !team.departmentId)
  return (
    <section className="org-edit" aria-label={t('teams.edit')}>
      <p className="hint">{t('orgEdit.lead')}</p>
      <form
        className="row-actions org-edit-add"
        onSubmit={(event) => {
          event.preventDefault()
          setBusy(true)
          setMessages([])
          void api.createDepartment(workspaceId, { name: name.trim(), parentId: parentId || null })
            .then(async () => { setName(''); setParentId(''); await refresh() })
            .catch((caught) => setMessages([failureReason(caught, t)]))
            .finally(() => setBusy(false))
        }}
      >
        <Field label={t('teams.departmentName')}>
          <input ref={nameInput} required value={name} onChange={(event) => setName(event.target.value)} />
        </Field>
        <Field label={t('teams.departmentParent')}>
          <Select value={parentId} onChange={(event) => setParentId(event.target.value)}>
            <option value="">{t('orgEdit.top')}</option>
            {departments.map((department) => <option key={department.id} value={department.id}>{department.name}</option>)}
          </Select>
        </Field>
        <Button type="submit" disabled={busy}>{t('orgEdit.addDepartment')}</Button>
      </form>
      {roots.map(renderDepartment)}
      <section
        className={drag?.kind === 'team' ? 'org-edit-zone is-target' : 'org-edit-zone'}
        onDragOver={(event) => allow(event, drag?.kind === 'team')}
        onDrop={(event) => drop(event, { department: null })}
      >
        <h2>{t('orgEdit.unplacedZone')}</h2>
        {unplaced.length === 0 ? <p className="hint">{t('orgEdit.empty')}</p> : null}
        <ul className="plain-list">
          {unplaced.map((team) => (
            <li key={team.id} className="org-edit-team" draggable onDragStart={() => setDrag({ kind: 'team', id: team.id })} onDragEnd={() => setDrag(null)}>
              <strong>{team.name}</strong>
              {moved('team', team.id) ? <Badge tone="accent">{t('orgEdit.moving')}</Badge> : null}
              <TeamPlace team={team} departments={effective.departments} onMove={(to) => moveTeam(team, to)} />
            </li>
          ))}
        </ul>
      </section>
      {drag?.kind === 'department' ? (
        <section className="org-edit-zone is-target" onDragOver={(event) => allow(event, true)} onDrop={(event) => drop(event, { department: null })}>
          <h2>{t('orgEdit.topZone')}</h2>
        </section>
      ) : null}
      {messages.length > 0 ? <ul className="plain-list" role="alert">{messages.map((message) => <li key={message} className="form-error">{message}</li>)}</ul> : null}
      {applied ? <p className="hint" role="status">{t('orgEdit.applied')}</p> : null}
      {moves.length > 0 ? (
        <div className="org-edit-changes" role="group" aria-label={t('orgEdit.changes', { count: String(moves.length) })}>
          <strong>{t('orgEdit.changes', { count: String(moves.length) })}</strong>
          <ul className="plain-list">
            {moves.map((move) => (
              <li key={`${move.kind}:${move.id}`} className="row-actions">
                <span>{t('orgEdit.change', { name: move.name, from: nameOf(move.from, move.kind), to: nameOf(move.to, move.kind) })}</span>
                <Button type="button" variant="ghost" disabled={busy} onClick={() => onMoves(moves.filter((item) => item !== move))}>{t('orgEdit.undo')}</Button>
              </li>
            ))}
          </ul>
          <div className="row-actions">
            {moves.length > 1 ? <Button type="button" variant="secondary" disabled={busy} onClick={() => onMoves([])}>{t('orgEdit.undoAll')}</Button> : null}
            <Button type="button" disabled={busy} onClick={() => void apply()}>{busy ? t('orgEdit.applying') : t('orgEdit.apply')}</Button>
          </div>
        </div>
      ) : null}
      {settings ? (
        <DepartmentSettings
          workspaceId={workspaceId}
          department={settings}
          parentName={departments.find((item) => item.id === settings.parentId)?.name ?? t('orgEdit.top')}
          locked={moves.length > 0}
          onClose={() => setSettings(null)}
          onChanged={refresh}
        />
      ) : null}
    </section>
  )
}

function TeamPlace({ team, departments, onMove }: { team: Team; departments: Department[]; onMove: (to: string | null) => void }) {
  const { t } = useI18n()
  return (
    <Select aria-label={t('orgEdit.moveTeam', { name: team.name })} value={team.departmentId ?? ''} onChange={(event) => onMove(event.target.value || null)}>
      <option value="">{t('orgEdit.noDepartment')}</option>
      {departments.map((department) => <option key={department.id} value={department.id}>{department.name}</option>)}
    </Select>
  )
}

/** Renames a department or deletes it: what is inside moves up one level. */
function DepartmentSettings({
  workspaceId,
  department,
  parentName,
  locked,
  onClose,
  onChanged,
}: {
  workspaceId: string
  department: Department
  parentName: string
  locked: boolean
  onClose: () => void
  onChanged: () => Promise<void>
}) {
  const { t } = useI18n()
  const [name, setName] = useState(department.name)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  async function run(action: () => Promise<unknown>, close: boolean) {
    setBusy(true)
    setError(null)
    try {
      await action()
      await onChanged()
      if (close) onClose()
    } catch {
      setError(t('error.generic'))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Dialog open title={t('orgEdit.settingsTitle', { name: department.name })} onClose={onClose}>
      <form
        onSubmit={(event) => {
          event.preventDefault()
          void run(() => api.updateDepartment(workspaceId, department.id, { name: name.trim(), parentId: department.parentId }), true)
        }}
      >
        <Field label={t('orgEdit.rename')}>
          <input required value={name} onChange={(event) => setName(event.target.value)} />
        </Field>
        <Button type="submit" disabled={busy || name.trim() === '' || name.trim() === department.name}>{t('orgEdit.renameSave')}</Button>
      </form>
      <h3>{t('orgEdit.deleteTitle')}</h3>
      <p className="hint">{t('orgEdit.deleteBody', { parent: parentName })}</p>
      {locked ? <p className="hint">{t('orgEdit.deleteNeedsApply')}</p> : null}
      {error ? <p className="form-error" role="alert">{error}</p> : null}
      <Button type="button" variant="danger" disabled={busy || locked} onClick={() => void run(() => api.deleteDepartment(workspaceId, department.id), true)}>{t('orgEdit.delete')}</Button>
    </Dialog>
  )
}
