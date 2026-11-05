import { useQuery } from '@tanstack/react-query'
import { useParams } from 'react-router'
import { useI18n } from '../i18n/I18nProvider'
import { queries } from '../queries'
import { PersonRow, useStartDm } from '../search/DirectoryResults'

export function OrganizationPage() {
  const { t } = useI18n()
  const { workspaceId = '', organizationId = '' } = useParams()
  const directory = useQuery({ ...queries.organization(workspaceId, organizationId), enabled: workspaceId.length > 0 && organizationId.length > 0 })
  const start = useStartDm(workspaceId)
  const organization = directory.data?.organization

  return (
    <div className="page-scroll">
      {directory.isError ? <p className="form-error">{t('error.generic')}</p> : null}
      {organization ? (
        <header className="page-intro">
          <h1 data-directory-org={organization.name}>{organization.name}</h1>
          <p>
            {organization.timeZone}
            {' · '}
            {t('search.memberCount').replace('{count}', String(organization.memberCount))}
          </p>
        </header>
      ) : null}
      <h2>{t('directory.members')}</h2>
      <ul className="directory-list">
        {directory.data?.members.map((person) => (
          <li key={person.id}>
            <PersonRow
              person={person}
              selected={false}
              selectable={false}
              onToggle={() => undefined}
              onOpen={() => void start.open([person.id])}
            />
          </li>
        ))}
      </ul>
      {start.error ? <p className="form-error">{t('error.generic')}</p> : null}
    </div>
  )
}
