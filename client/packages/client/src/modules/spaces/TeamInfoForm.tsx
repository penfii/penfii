import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import type { Team } from '@penfii/contracts'
import { isApiError } from '@penfii/api-client'
import { api } from '../../api'
import { Button, Field } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'

/** Name and plain-text description of a Team. No counter: the product sets no length limit. */
export function TeamInfoForm({ workspaceId, team }: { workspaceId: string; team: Team }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [name, setName] = useState(team.name)
  const [description, setDescription] = useState(team.description ?? '')
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  const [busy, setBusy] = useState(false)
  const unchanged = name.trim() === team.name && description.trim() === (team.description ?? '')
  async function save() {
    if (busy || !name.trim() || unchanged) return
    setBusy(true)
    setError(null)
    setSaved(false)
    try {
      await api.updateTeam(workspaceId, team.id, { name: name.trim(), description: description.trim() === '' ? null : description.trim() })
      await queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })
      setSaved(true)
    } catch (caught) {
      if (isApiError(caught) && caught.code === 'validation') setError(t('teams.descriptionTooLong'))
      else if (isApiError(caught) && caught.code === 'team_archived') setError(t('teams.error.archived'))
      else setError(t('error.generic'))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="card settings-card team-info-form" onSubmit={(event) => { event.preventDefault(); void save() }}>
      <h2>{t('teams.info')}</h2>
      <Field label={t('teams.rename')}>
        <input value={name} onChange={(event) => { setName(event.target.value); setSaved(false) }} />
      </Field>
      <Field label={t('teams.description')} hint={t('teams.descriptionHint')}>
        <textarea rows={3} value={description} onChange={(event) => { setDescription(event.target.value); setSaved(false) }} />
      </Field>
      <p className="hint">{t('teams.infoWho')}</p>
      {error ? <p className="field-error" role="alert">{error}</p> : null}
      {saved ? <p role="status" className="hint">{t('teams.infoSaved')}</p> : null}
      <Button type="submit" disabled={busy || !name.trim() || unchanged}>{t('teams.infoSave')}</Button>
    </form>
  )
}
