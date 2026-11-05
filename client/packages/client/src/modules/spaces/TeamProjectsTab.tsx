import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router'
import type { TeamProject } from '@penfii/contracts'
import { Badge } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'

/** The Projects a Team is linked to: the Team's role, and only the viewer's own access. Roles are changed in the Project. */
export function TeamProjectsTab({ workspaceId, teamId }: { workspaceId: string; teamId: string }) {
  const { t } = useI18n()
  const projects = useQuery(queries.teamProjects(workspaceId, teamId))
  if (projects.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (projects.isError) return <p className="form-error" role="alert">{t('error.generic')}</p>
  return (
    <section className="team-projects" aria-label={t('teams.tab.projects')}>
      <p className="hint">{t('teams.projects.whyReadOnly')}</p>
      {projects.data.length === 0 ? <p className="hint">{t('teams.projects.empty')}</p> : (
        <ul className="plain-list team-project-list">
          {projects.data.map((project) => <TeamProjectRow key={project.projectId} workspaceId={workspaceId} project={project} />)}
        </ul>
      )}
    </section>
  )
}

function TeamProjectRow({ workspaceId, project }: { workspaceId: string; project: TeamProject }) {
  const { t } = useI18n()
  const teamRole = project.teamRole ? t('teams.projects.teamRole', { role: t(`permissions.role.${project.teamRole}`) }) : t('teams.projects.linkedOnly')
  let mine: string
  if (!project.canOpen || !project.myRole) mine = project.canOpen ? t('teams.projects.mineNone') : t('teams.projects.noAccess')
  else {
    const source = project.mySources.map((item) => t(`teams.projects.source.${item}`)).join(' · ')
    mine = t('teams.projects.mine', { role: t(`permissions.role.${project.myRole}`), source })
  }
  return (
    <li className="team-project-row">
      <span className="team-project-head">
        {project.canOpen ? <Link to={`/w/${workspaceId}/projects/${project.projectId}`}><strong>{project.name}</strong></Link> : <strong>{project.name}</strong>}
        <Badge tone={project.teamRole ? 'accent' : 'neutral'}>{teamRole}</Badge>
        {project.archivedAt ? <Badge>{t('teams.projects.archivedProject')}</Badge> : null}
      </span>
      <span className="hint team-project-mine">{mine}</span>
      {project.canOpen ? <small className="hint">{project.mySources.length > 1 ? `${t('teams.projects.higher')} ` : ''}<Link to={`/w/${workspaceId}/projects/${project.projectId}/settings`}>{t('teams.projects.changeAt')}</Link></small> : null}
    </li>
  )
}
