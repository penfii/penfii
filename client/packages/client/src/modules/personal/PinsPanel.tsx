import { useState, type FormEvent } from 'react'
import { Link, useParams } from 'react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '../../api'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { CloseIcon } from '../../design/icons'

/** Shared Pins of a Project: what everyone in it should find first. Only a Project admin changes them. */
export function PinsPanel({ projectId }: { projectId: string }) {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const queryClient = useQueryClient()
  const pins = useQuery(queries.projectPins(projectId))
  const [url, setUrl] = useState('')
  const [title, setTitle] = useState('')
  const [failed, setFailed] = useState(false)
  if (!pins.data) return null
  const { canManage } = pins.data
  if (pins.data.pins.length === 0 && !canManage) return null

  async function refresh() {
    await queryClient.invalidateQueries({ queryKey: queryKeys.projectPins(projectId) })
  }
  async function add(event: FormEvent) {
    event.preventDefault()
    setFailed(false)
    try {
      await api.addProjectPin(projectId, { url: url.trim(), title: title.trim() || null })
      setUrl('')
      setTitle('')
    } catch {
      setFailed(true)
    }
    await refresh()
  }
  return (
    <section className="pins-panel" aria-label={t('pins.title')}>
      <h2>{t('pins.title')}</h2>
      <p className="hint">{t('pins.lead')}</p>
      {pins.data.pins.length === 0 ? <p className="hint">{t('pins.empty')}</p> : null}
      <ul className="pin-list">
        {pins.data.pins.map((pin) => (
          <li key={pin.id}>
            {pin.ticket ? (
              <Link to={`/w/${workspaceId}/projects/${projectId}/work?panel=ticket:${pin.ticket.id}`}>{pin.title}</Link>
            ) : (
              <a href={pin.url ?? '#'} target="_blank" rel="noreferrer noopener">{pin.title || pin.url}</a>
            )}
            {canManage ? (
              <button type="button" className="icon-button" aria-label={t('pins.remove', { title: pin.title })} onClick={() => void api.removePin(pin.id).finally(refresh)}><CloseIcon /></button>
            ) : null}
          </li>
        ))}
      </ul>
      {canManage ? (
        <form className="planning-form" onSubmit={(event) => void add(event)}>
          <input aria-label={t('pins.url')} placeholder="https://" value={url} onChange={(event) => setUrl(event.target.value)} />
          <input aria-label={t('pins.linkTitle')} placeholder={t('pins.linkTitle')} value={title} maxLength={120} onChange={(event) => setTitle(event.target.value)} />
          <Button type="submit" variant="secondary" disabled={!url.trim()}>{t('pins.add')}</Button>
          {failed ? <p className="board-error" role="alert">{t('pins.error')}</p> : null}
        </form>
      ) : null}
    </section>
  )
}
