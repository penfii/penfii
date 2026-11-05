import { useState } from 'react'
import { Link, useParams } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import type { ScopeLens } from '@penfii/contracts'
import { Badge, EmptyState, Tabs } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { FavoriteButton } from './SavedButtons'
import { Work } from '../work/Work'
import { isArchivedTeam } from '../spaces/teamState'

/** The Tickets of one Team, across its Projects, as a List or a Table. The place decides the scope. */
export function TeamWorkPage() {
  const { t } = useI18n()
  const { workspaceId = '', teamId = '' } = useParams()
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  const team = space.data?.teams.find((item) => item.id === teamId)
  if (space.isSuccess && !team) return <EmptyState title={t('notFound')} body={t('work.team.notFound')} />
  return (
    <div className="page-scroll board-page">
      <header className="page-intro">
        <p className="hint">{t('work.team.label')}</p>
        <h1>
          {team?.name ?? t('loading')}{isArchivedTeam(team) ? <> <Badge>{t('teams.archivedTag')}</Badge></> : null} <FavoriteButton workspaceId={workspaceId} kind="TEAM" targetId={teamId} label={team?.name} />
        </h1>
      </header>
      <Link to={`/w/${workspaceId}/directory/teams/${teamId}`}>{t('teams.open')}</Link>
      <Work target={{ kind: 'TEAM', workspaceId, teamId }} title={team?.name} />
    </div>
  )
}

/**
 * My Work: the Tickets that are mine now, as the Workspace scope of Work. The lens only narrows the
 * dataset; there is no separate list. The 관찰 lens lists the Tickets the person follows.
 */
export function MyWorkPage() {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const [lens, setLens] = useState<ScopeLens>('ASSIGNED')
  return (
    <div className="page-scroll board-page">
      <header className="page-intro">
        <h1>{t('mywork.title')}</h1>
        <p>{t('mywork.lead')}</p>
      </header>
      <div className="mywork-lens">
        <Tabs
          tabs={[
            { id: 'ASSIGNED', label: t('mywork.lens.ASSIGNED') },
            { id: 'CREATED', label: t('mywork.lens.CREATED') },
            { id: 'WATCHING', label: t('mywork.lens.WATCHING') },
          ]}
          value={lens}
          onChange={(id) => setLens(id as ScopeLens)}
        />
      </div>
      <Work key={lens} target={{ kind: 'WORKSPACE', workspaceId }} lens={lens} title={t('mywork.title')} />
    </div>
  )
}
