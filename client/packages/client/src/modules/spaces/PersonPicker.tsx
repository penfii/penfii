import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Field } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'

const SHOWN = 8

export type PickedPerson = { id: string; displayName: string }

/** Finds people of the Workspace by name and offers the ones that are not already in `taken`. */
export function PersonPicker({
  workspaceId,
  taken,
  label,
  onPick,
  preferUnaffiliated = false,
}: {
  workspaceId: string
  taken: ReadonlySet<string>
  label: string
  onPick: (person: PickedPerson) => void
  /** Puts people who are on no Team first and marks them. */
  preferUnaffiliated?: boolean
}) {
  const { t } = useI18n()
  const [query, setQuery] = useState('')
  const results = useQuery({ ...queries.directorySearch(workspaceId, query.trim(), 'person'), enabled: workspaceId.length > 0 })
  const everyone = useQuery({ ...queries.people(workspaceId), enabled: preferUnaffiliated && workspaceId.length > 0 })
  const unaffiliated = new Set((everyone.data ?? []).filter((person) => person.state === 'ACTIVE' && person.teams.length === 0).map((person) => person.userId))
  const hits = (results.data?.results ?? []).flatMap((hit) => (hit.type === 'person' && hit.person && !taken.has(hit.person.id) ? [hit.person] : []))
  const found = preferUnaffiliated ? [...hits.filter((person) => unaffiliated.has(person.id)), ...hits.filter((person) => !unaffiliated.has(person.id))] : hits
  const people = found.slice(0, SHOWN)
  return (
    <div className="person-picker">
      <Field label={label}>
        <input value={query} onChange={(event) => setQuery(event.target.value)} />
      </Field>
      {results.isSuccess && people.length === 0 ? <p className="hint">{t('picker.none')}</p> : null}
      {found.length > SHOWN ? <p className="hint">{t('picker.more')}</p> : null}
      <ul className="directory-list">
        {people.map((person) => (
          <li key={person.id}>
            <button type="button" className="directory-row" onClick={() => { onPick({ id: person.id, displayName: person.displayName }); setQuery('') }}>
              <strong>{person.displayName}</strong>
              {person.username ? <small> @{person.username}</small> : null}
              {preferUnaffiliated && unaffiliated.has(person.id) ? <small className="hint"> · {t('picker.unaffiliated')}</small> : null}
            </button>
          </li>
        ))}
      </ul>
    </div>
  )
}
