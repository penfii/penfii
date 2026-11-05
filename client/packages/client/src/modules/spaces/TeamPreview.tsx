import { useEffect, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router'
import type { Department, Team } from '@penfii/contracts'
import { Badge, Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { departmentPath } from './orgTree'
import { isArchivedTeam } from './teamState'

const FIRST = 4

/** A glance at one Team from the organization chart: where it sits, who is on it, and the way to its page. Every member may read it. */
export function TeamPreview({ workspaceId, team, departments, onClose }: { workspaceId: string; team: Team; departments: Department[]; onClose: () => void }) {
  const { t } = useI18n()
  const roster = useQuery(queries.adminTeamMembers(workspaceId, team.id))
  const [all, setAll] = useState(false)
  const rootRef = useRef<HTMLElement>(null)
  const onCloseRef = useRef(onClose)
  onCloseRef.current = onClose
  const path = departmentPath(departments, team.departmentId ?? null)
  const members = roster.data ?? []
  const shown = all ? members : members.slice(0, FIRST)
  // Escape closes the preview while focus is inside it, and focus returns to the card that opened it.
  useEffect(() => {
    const root = rootRef.current
    root?.querySelector('button')?.focus()
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || !root?.contains(event.target as Node)) return
      event.preventDefault()
      onCloseRef.current()
    }
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('keydown', onKey)
      const label = root?.getAttribute('aria-label')
      const active = document.activeElement
      const back = label ? document.querySelector(`button.team-card-name[aria-label="${CSS.escape(label)}"]`) : null
      if (back instanceof HTMLElement && (active === document.body || active === null || (root?.contains(active) ?? false))) back.focus()
    }
  }, [team.id])
  return (
    <aside ref={rootRef} className="card org-preview" aria-label={t('org.preview', { name: team.name })}>
      <header className="org-preview-head">
        <h2>
          {team.name}
          {isArchivedTeam(team) ? <> <Badge>{t('teams.archivedTag')}</Badge></> : null}
        </h2>
        <Button type="button" variant="ghost" aria-label={t('org.previewClose')} onClick={onClose}>×</Button>
      </header>
      {path.length > 0 ? <p className="hint">{path.join(' / ')}</p> : null}
      {team.description ? <p className="team-description">{team.description}</p> : null}
      {roster.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {roster.isError ? <p className="form-error" role="alert">{t('error.generic')}</p> : null}
      {roster.isSuccess ? (
        <>
          <p className="hint">{t('org.members', { count: String(members.length) })}</p>
          <ul className="plain-list">
            {shown.map((member) => (
              <li key={member.userId} className="org-preview-member">
                <span>{member.displayName}</span>
                {member.role === 'MANAGER' ? <Badge tone="accent">{t('org.manager')}</Badge> : null}
              </li>
            ))}
          </ul>
          {members.length > FIRST ? (
            <Button type="button" variant="ghost" onClick={() => setAll((value) => !value)}>
              {all ? t('org.fewer') : t('org.more', { count: String(members.length - FIRST) })}
            </Button>
          ) : null}
        </>
      ) : null}
      <Link to={`/w/${workspaceId}/directory/teams/${team.id}`}>{t('teams.open')}</Link>
    </aside>
  )
}
