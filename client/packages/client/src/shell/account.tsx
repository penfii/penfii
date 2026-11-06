import { useEffect, useMemo, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isNotFound } from '@penfii/api-client'
import { useNavigate } from 'react-router'
import { api } from '../api'
import { Button, Dialog, Field } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import { queryKeys } from '../queryKeys'
import { queries } from '../queries'
import { errorText } from '../ui/errorText'

/** Where the last workspace is remembered for the session, so the entry pages can return to it. */
export const LAST_WORKSPACE_KEY = 'penfii.lastWorkspace'

/**
 * The workspace a screen is in: the list the switcher shows, the Space, and whether the id in the
 * address is one the person may open at all. Both shells use it, so a wrong id is treated alike.
 */
export function useActiveWorkspace(workspaceId: string | undefined) {
  const workspaces = useQuery(queries.workspaces())
  const space = useQuery({ ...queries.space(workspaceId ?? ''), enabled: Boolean(workspaceId) })
  useEffect(() => {
    if (!workspaceId) return
    try {
      sessionStorage.setItem(LAST_WORKSPACE_KEY, workspaceId)
    } catch {
      // The switcher still works from the loaded workspace list.
    }
  }, [workspaceId])
  const workspace = workspaces.data?.find((item) => item.id === workspaceId)
  const notFound = !workspaceId || (workspaces.isSuccess && !workspace) || (space.isError && isNotFound(space.error))
  return { workspaces, workspace, space, notFound }
}

/** "Quiet for one hour": turns Do Not Disturb on until an hour from now. Resolves true when it was saved. */
export function useSnooze(): () => Promise<boolean> {
  const queryClient = useQueryClient()
  const prefs = useQuery(queries.notificationPreferences())
  return async () => {
    const current = prefs.data
    if (!current) return false
    try {
      await api.saveNotificationPreferences({
        level: current.level,
        dnd: { ...current.dnd, enabled: true, snoozeUntil: new Date(Date.now() + 3_600_000).toISOString() },
        external: current.external,
        expectedVersion: current.version,
      })
      await queryClient.invalidateQueries({ queryKey: queryKeys.notificationPreferences })
      return true
    } catch {
      // Preferences stay as they were; the notifications page reports a conflict.
      return false
    }
  }
}

/** The help text of the account menu. */
export function HelpDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t } = useI18n()
  return (
    <Dialog open={open} title={t('help.open')} onClose={onClose}>
      <p>{t('help.body')}</p>
    </Dialog>
  )
}

export function StatusDialog({
  open,
  onClose,
  displayName,
  jobTitle,
  timeZone,
  statusEmoji,
  statusText,
  statusClearAt,
}: {
  open: boolean
  onClose: () => void
  displayName: string
  jobTitle: string
  timeZone: string
  statusEmoji: string
  statusText: string
  statusClearAt: string
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [emoji, setEmoji] = useState(statusEmoji)
  const [text, setText] = useState(statusText)
  const [clearAt, setClearAt] = useState(statusClearAt ? toLocalInput(statusClearAt) : '')
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (!open) return
    setEmoji(statusEmoji)
    setText(statusText)
    setClearAt(statusClearAt ? toLocalInput(statusClearAt) : '')
    setError(null)
  }, [open, statusEmoji, statusText, statusClearAt])
  return (
    <Dialog open={open} title={t('menu.status')} onClose={onClose} presentation="full">
      <form
        onSubmit={(event) => {
          event.preventDefault()
          setError(null)
          void api.updateProfile({
            displayName,
            jobTitle: jobTitle || null,
            timeZone: timeZone || null,
            statusEmoji: emoji.trim() || null,
            statusText: text.trim() || null,
            statusClearAt: clearAt ? new Date(clearAt).toISOString() : null,
          }).then(async () => {
            await queryClient.invalidateQueries({ queryKey: queryKeys.me })
            onClose()
          }).catch((caught: unknown) => setError(errorText(caught, t('error.generic'))))
        }}
      >
        <Field label={t('profile.status')}>
          <input value={emoji} onChange={(event) => setEmoji(event.target.value)} maxLength={16} />
        </Field>
        <Field label={t('profile.statusText')}>
          <input value={text} onChange={(event) => setText(event.target.value)} maxLength={80} />
        </Field>
        <Field label={t('profile.statusClear')}>
          <input type="datetime-local" value={clearAt} onChange={(event) => setClearAt(event.target.value)} />
        </Field>
        {error ? <p className="form-error">{error}</p> : null}
        <Button type="submit">{t('account.save')}</Button>
      </form>
    </Dialog>
  )
}

function toLocalInput(iso: string): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return ''
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}

export function WorkspaceCreate({ buttonClassName }: { buttonClassName?: string }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const creation = useQuery(queries.workspaceCreation())
  const [open, setOpen] = useState(false)
  const [name, setName] = useState('')
  const [timeZone, setTimeZone] = useState(Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC')
  const [error, setError] = useState<string | null>(null)
  const zones = useMemo(() => {
    const all = Intl.supportedValuesOf('timeZone')
    const current = Intl.DateTimeFormat().resolvedOptions().timeZone
    return current && !all.includes(current) ? [current, ...all] : all
  }, [])
  // Only the first Workspace of an instance can be created; afterwards the server refuses.
  if (!creation.data) return null
  return (
    <>
      <button type="button" className={buttonClassName} onClick={() => setOpen(true)}>{t('workspaces.create')}</button>
      <Dialog open={open} title={t('workspaces.create')} onClose={() => setOpen(false)} presentation="full">
        <form
          onSubmit={(event) => {
            event.preventDefault()
            setError(null)
            void api.createWorkspace({ name: name.trim(), timeZone }).then(async (created) => {
              setName('')
              await queryClient.invalidateQueries({ queryKey: queryKeys.workspaces })
              setOpen(false)
              void navigate(`/w/${created.id}`)
            }).catch((caught: unknown) => setError(errorText(caught, t('error.generic'))))
          }}
        >
          <Field label={t('workspaces.name')}>
            <input required value={name} onChange={(event) => setName(event.target.value)} />
          </Field>
          <Field label={t('workspaces.timeZone')}>
            <select value={timeZone} onChange={(event) => setTimeZone(event.target.value)}>
              {zones.map((zone) => (
                <option key={zone} value={zone}>{zone}</option>
              ))}
            </select>
          </Field>
          {error ? <p className="form-error">{error}</p> : null}
          <Button type="submit">{t('workspaces.create')}</Button>
        </form>
      </Dialog>
    </>
  )
}
