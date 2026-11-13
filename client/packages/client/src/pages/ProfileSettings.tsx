import { useEffect, useState, type FormEvent } from 'react'
import { Link } from 'react-router'
import { isApiError } from '@penfii/api-client'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '../api'
import { AvatarCropDialog } from '../design/AvatarCropDialog'
import { Avatar, Button, Field } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import { useSession } from '../modules/identity'
import { queries } from '../queries'
import { queryKeys } from '../queryKeys'
import { errorText } from '../ui/errorText'

const PRESETS = [
  ['💬', 'profile.statusMeeting'],
  ['🍱', 'profile.statusLunch'],
  ['🌴', 'profile.statusAway'],
  ['🏠', 'profile.statusHome'],
] as const

function zones(): string[] {
  const all = Intl.supportedValuesOf('timeZone')
  const current = Intl.DateTimeFormat().resolvedOptions().timeZone
  return current && !all.includes(current) ? [current, ...all] : all
}

export function ProfileSettings({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const workspaceRole = useQuery(queries.workspaces()).data?.find((item) => item.id === workspaceId)?.role
  const people = useQuery(queries.people(workspaceId))
  const session = useSession()
  const queryClient = useQueryClient()
  const me = session.data
  const [displayName, setDisplayName] = useState('')
  const [jobTitle, setJobTitle] = useState('')
  const [timeZone, setTimeZone] = useState('')
  const [statusEmoji, setStatusEmoji] = useState('')
  const [statusText, setStatusText] = useState('')
  const [statusClear, setStatusClear] = useState('')
  const [photoKey, setPhotoKey] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  const [busy, setBusy] = useState(false)
  const [cropFile, setCropFile] = useState<File | null>(null)

  useEffect(() => {
    if (!me) return
    setDisplayName(me.displayName)
    setJobTitle(me.jobTitle ?? '')
    setTimeZone(me.timeZone ?? Intl.DateTimeFormat().resolvedOptions().timeZone ?? 'UTC')
    setStatusEmoji(me.statusEmoji ?? '')
    setStatusText(me.statusText ?? '')
    setStatusClear(me.statusClearAt ? toLocalInput(me.statusClearAt) : '')
  }, [me])

  async function onSubmit(event: FormEvent) {
    event.preventDefault()
    setBusy(true)
    setError(null)
    setSaved(false)
    try {
      await api.updateProfile({
        displayName: displayName.trim(),
        jobTitle: jobTitle.trim() || null,
        timeZone: timeZone || null,
        statusEmoji: statusEmoji.trim() || null,
        statusText: statusText.trim() || null,
        statusClearAt: statusClear ? new Date(statusClear).toISOString() : null,
      })
      setSaved(true)
      await queryClient.invalidateQueries({ queryKey: queryKeys.me })
    } catch (caught) {
      setError(errorText(caught, t('error.generic'), t('error.dbUnavailable')))
    } finally {
      setBusy(false)
    }
  }

  async function onPhoto(file: File | undefined) {
    if (!file) return
    setError(null)
    if (!file.type.startsWith('image/')) {
      setError(t('profile.photoError'))
      return
    }
    setCropFile(file)
  }

  async function onCropped(blob: Blob) {
    setCropFile(null)
    setBusy(true)
    setError(null)
    try {
      await api.uploadAvatar(blob)
      setPhotoKey(Date.now())
      await queryClient.invalidateQueries({ queryKey: queryKeys.me })
    } catch (caught) {
      if (isApiError(caught)) setError(errorText(caught, t('profile.photoError')))
      else setError(t('profile.photoError'))
    } finally {
      setBusy(false)
    }
  }

  if (!me) return <p className="hint">{t('loading')}</p>
  const teamNames = people.data?.find((person) => person.userId === me.id)?.teams.map((team) => team.name) ?? []
  const crop = cropFile ? (
    <AvatarCropDialog file={cropFile} onCancel={() => setCropFile(null)} onConfirm={(blob) => void onCropped(blob)} />
  ) : null
  const statusLine = [statusEmoji, statusText].filter(Boolean).join(' ')
  return (
    <div className="page-scroll profile-page">
      {crop}
      <header className="page-intro">
        <h1>{t('settings.profile')}</h1>
        <p className="hint">{t('profile.lead')}</p>
      </header>
      <div className="profile-layout">
        <form className="card settings-card" onSubmit={(event) => void onSubmit(event)}>
          <div className="profile-photo">
            <Avatar name={displayName || me.displayName} userId={me.id} hasAvatar={me.hasAvatar} photoKey={photoKey || undefined} />
            <div>
              <strong>{t('profile.photo')}</strong>
              <p className="hint">{t('profile.photoHint')}</p>
              <div className="row-actions">
                <label className="btn btn-secondary">
                  {t('profile.upload')}
                  <input
                    type="file"
                    accept="image/jpeg,image/png,image/webp,image/gif"
                    hidden
                    onChange={(event) => {
                      const file = event.target.files?.[0]
                      event.target.value = ''
                      void onPhoto(file)
                    }}
                  />
                </label>
                {me.hasAvatar ? (
                  <Button
                    type="button"
                    variant="ghost"
                    disabled={busy}
                    onClick={() => {
                      setBusy(true)
                      void api.deleteAvatar().then(async () => {
                        setPhotoKey(Date.now())
                        await queryClient.invalidateQueries({ queryKey: queryKeys.me })
                      }).catch((caught: unknown) => setError(errorText(caught, t('error.generic'), t('error.dbUnavailable')))).finally(() => setBusy(false))
                    }}
                  >
                    {t('profile.remove')}
                  </Button>
                ) : null}
              </div>
            </div>
          </div>
          <Field label={t('profile.displayName')} hint={t('profile.displayNameHint')}>
            <input required value={displayName} onChange={(event) => setDisplayName(event.target.value)} />
          </Field>
          <Field label={t('profile.mention')} hint={t('profile.mentionHint')}>
            <input readOnly value={displayName.trim() ? `@${displayName.trim()}` : ''} />
          </Field>
          <Field label={t('profile.jobTitle')}>
            <input value={jobTitle} maxLength={80} onChange={(event) => setJobTitle(event.target.value)} />
          </Field>
          <Field label={t('profile.timeZone')} hint={t('profile.timeZoneHint')}>
            <select value={timeZone} onChange={(event) => setTimeZone(event.target.value)}>
              {zones().map((zone) => <option key={zone} value={zone}>{zone}</option>)}
            </select>
          </Field>
          <fieldset>
            <legend>{t('profile.status')}</legend>
            <div className="row-actions">
              {PRESETS.map(([emoji, key]) => (
                <Button
                  key={key}
                  type="button"
                  variant="secondary"
                  onClick={() => {
                    setStatusEmoji(emoji)
                    setStatusText(t(key))
                  }}
                >
                  {emoji} {t(key)}
                </Button>
              ))}
              <Button type="button" variant="ghost" onClick={() => { setStatusEmoji(''); setStatusText(''); setStatusClear('') }}>
                {t('profile.statusNone')}
              </Button>
            </div>
            <Field label={t('profile.statusText')}>
              <input value={statusText} maxLength={80} onChange={(event) => setStatusText(event.target.value)} />
            </Field>
            <Field label={t('profile.statusClear')} hint={t('profile.statusClearHint')}>
              <input type="datetime-local" value={statusClear} onChange={(event) => setStatusClear(event.target.value)} />
            </Field>
          </fieldset>
          <p className="profile-email">
            <span>{t('account.email')}: {me.email}</span>
            <Link to={`/w/${workspaceId}/settings/account`}>{t('profile.emailHint')}</Link>
          </p>
          {error ? <p className="form-error" role="alert">{error}</p> : null}
          {saved ? <p role="status">{t('account.saved')}</p> : null}
          <Button type="submit" disabled={busy}>{t('account.save')}</Button>
        </form>
        <aside className="card settings-card profile-preview">
          <h2>{t('profile.preview')}</h2>
          <div className="profile-preview-card">
            <Avatar name={displayName || me.displayName} userId={me.id} hasAvatar={me.hasAvatar} photoKey={photoKey || undefined} />
            <div>
              <strong>{displayName || me.displayName}</strong>
              <p className="hint">@{displayName.trim() || me.displayName}</p>
              {jobTitle.trim() ? <p>{jobTitle.trim()}</p> : null}
              {statusLine ? <p>{statusLine}</p> : null}
              <p className="hint">{me.email}</p>
            </div>
          </div>
        </aside>
        <aside className="card settings-card profile-fixed">
          <h2>{t('profile.fixed')}</h2>
          <dl>
            <div><dt>{t('profile.fixedRole')}</dt><dd>{workspaceRole === 'OWNER' ? t('role.owner') : workspaceRole === 'ADMIN' ? t('role.admin') : workspaceRole ? t('role.member') : '—'}</dd></div>
            <div><dt>{t('profile.fixedTeams')}</dt><dd>{teamNames.length > 0 ? teamNames.join(', ') : t('profile.fixedNoTeam')}</dd></div>
          </dl>
          <p className="hint">{t('profile.fixedHint')}</p>
        </aside>
      </div>
    </div>
  )
}

function toLocalInput(iso: string): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return ''
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}
