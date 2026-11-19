import { NavLink } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import { useI18n } from '../i18n/I18nProvider'
import { useSession } from '../modules/identity'
import { activeTeams } from '../modules/spaces/teamState'
import { queries } from '../queries'

/**
 * The Teams section of the side list. It lives apart from the Teams pages on purpose: the shell shows it
 * on every screen, while the pages (org chart, all Teams, member directory, Team detail) load only when opened.
 *
 * Three places are fixed (the org chart, all Teams, all members); below them are the Teams the person is really on,
 * whatever department they sit in. Browsing every Team, creating one and finding people without a Team are done in
 * those places, not from this list.
 */
export function TeamsNav({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const session = useSession()
  const space = useQuery(queries.space(workspaceId))
  const people = useQuery({ ...queries.people(workspaceId), enabled: workspaceId.length > 0 })
  const me = (people.data ?? []).find((person) => person.userId === session.data?.id)
  const mineIds = new Set(me?.teams.map((team) => team.id) ?? [])
  const mine = activeTeams(space.data?.teams ?? [])
    .filter((team) => mineIds.has(team.id))
    .sort((a, b) => a.name.localeCompare(b.name))
  const base = `/w/${workspaceId}/directory`
  return (
    <>
      <ul>
        <li><NavLink end className="tree-link" to={base}>{t('teams.org')}</NavLink></li>
        <li><NavLink end className="tree-link" to={`${base}/teams`}>{t('teams.all')}</NavLink></li>
        <li><NavLink className="tree-link" to={`${base}/members`}>{t('teams.members')}</NavLink></li>
      </ul>
      <section className="tree-block">
        <h3>{t('teams.mineTitle')} {mine.length > 0 ? <span className="list-count">{mine.length}</span> : null}</h3>
        {mine.length === 0 ? (
          <p className="tree-empty">{people.isPending ? t('loading') : t('teams.nav.noMine')}</p>
        ) : (
          <ul>
            {mine.map((team) => (
              <li key={team.id}><NavLink className="tree-link" to={`${base}/teams/${team.id}`}>{team.name}</NavLink></li>
            ))}
          </ul>
        )}
      </section>
    </>
  )
}
