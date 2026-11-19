import { useEffect, useId, useRef, useState, type FormEvent } from 'react'
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
import { labelProblem } from '../ui/labelRule'
import { UnsavedBlocker, type UnsavedCopy } from '../ui/UnsavedBlocker'
import {
  changedGroups, groupSlice, mergeLatest, payloadFor, takeLatest, valuesAfterSave, valuesFromMe,
  type Conflict, type Conflicts, type Group, type ProfileValues,
} from './profileForm'

const PRESETS = [
  ['💬', 'profile.statusMeeting'],
  ['🍱', 'profile.statusLunch'],
  ['🌴', 'profile.statusAway'],
  ['🏠', 'profile.statusHome'],
] as const

const LEAVE_COPY: UnsavedCopy = {
  title: 'profile.leave.title',
  body: 'profile.leave.body',
  keep: 'profile.leave.keep',
  leave: 'profile.leave.discard',
  saveAndLeave: 'profile.leave.save',
}

function zones(): string[] {
  const all = Intl.supportedValuesOf('timeZone')
  const current = Intl.DateTimeFormat().resolvedOptions().timeZone
  return current && !all.includes(current) ? [current, ...all] : all
}

/** The server refuses control characters in every profile text (a pasted tab is one). */
const CONTROL = /\p{Cc}/u

type Feedback = { kind: 'saved'; at: Date } | { kind: 'failed' } | { kind: 'conflict' }

export function ProfileSettings({ workspaceId }: { workspaceId: string }) {
  const { t, locale } = useI18n()
  const workspaceRole = useQuery(queries.workspaces()).data?.find((item) => item.id === workspaceId)?.role
  const people = useQuery(queries.people(workspaceId))
  const session = useSession()
  const queryClient = useQueryClient()
  const me = session.data
  const fallbackZone = Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
  // `base` is what the server last told this tab; `form` is what is on screen. They differ only by what the person typed.
  const [form, setForm] = useState<ProfileValues | null>(null)
  const [base, setBase] = useState<ProfileValues | null>(null)
  const [conflicts, setConflicts] = useState<Conflicts>({})
  const [feedback, setFeedback] = useState<Feedback | null>(null)
  const [nameError, setNameError] = useState<string | null>(null)
  // Job title and status text: the server refuses control characters (a pasted tab) in them, so that is said under the field.
  const [textErrors, setTextErrors] = useState<Partial<Record<'jobTitle' | 'statusText', string>>>({})
  const kept = useRef<Partial<Record<Group, string>>>({})
  const formRef = useRef(form)
  const baseRef = useRef(base)
  const conflictsRef = useRef(conflicts)
  formRef.current = form
  baseRef.current = base
  conflictsRef.current = conflicts
  const nameRef = useRef<HTMLInputElement>(null)
  const jobTitleRef = useRef<HTMLInputElement>(null)
  const statusTextRef = useRef<HTMLInputElement>(null)
  const nameErrorId = useId()
  const jobTitleErrorId = useId()
  const statusTextErrorId = useId()
  const [photoKey, setPhotoKey] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [photoBusy, setPhotoBusy] = useState(false)
  const [cropFile, setCropFile] = useState<File | null>(null)

  // A new profile from the server never replaces what was typed: items left alone follow it, items being edited stay,
  // and an item changed on both sides is shown to the person (see mergeLatest).
  useEffect(() => {
    if (!me) return
    const next = valuesFromMe(me, fallbackZone)
    const current = baseRef.current
    if (!current || !formRef.current) {
      setForm(next)
      setBase(next)
      return
    }
    if (JSON.stringify(next) === JSON.stringify(current)) return
    const merged = mergeLatest(formRef.current, current, next, conflictsRef.current, kept.current)
    setForm(merged.form)
    setConflicts(merged.conflicts)
    setBase(next)
  }, [me, fallbackZone])

  const changed = form && base ? changedGroups(form, base) : []
  const dirty = changed.length > 0
  // A conflict matters only while that item still differs from the server's value.
  const open = (Object.keys(conflicts) as Group[]).filter((group) => changed.includes(group))

  // Leaving the page (reload, closing the tab) asks the browser's own question; leaving through the app asks UnsavedBlocker.
  useEffect(() => {
    if (!dirty) return
    const ask = (event: BeforeUnloadEvent) => event.preventDefault()
    window.addEventListener('beforeunload', ask)
    return () => window.removeEventListener('beforeunload', ask)
  }, [dirty])

  function edit(patch: Partial<ProfileValues>) {
    setForm((current) => (current ? { ...current, ...patch } : current))
    setFeedback(null)
    setNameError(null)
    setTextErrors({})
  }

  function revert() {
    if (!base) return
    kept.current = {}
    setForm(base)
    setConflicts({})
    setFeedback(null)
    setNameError(null)
    setTextErrors({})
  }

  function applyLatest(group: Group) {
    if (!form || !base) return
    setForm(takeLatest(form, base, group))
    setConflicts(({ [group]: _gone, ...rest }) => rest)
    setFeedback((current) => (current?.kind === 'conflict' ? null : current))
  }

  function keepMine(group: Group) {
    if (!base) return
    kept.current[group] = groupSlice(base, group)
    setConflicts(({ [group]: _gone, ...rest }) => rest)
  }

  function nameProblemText(value: string): string | null {
    const problem = labelProblem(value)
    if (!problem) return null
    return t(problem === 'blank' ? 'profile.nameBlank' : problem === 'tooLong' ? 'profile.nameTooLong' : 'profile.nameControl')
  }

  /** Saves what was changed. Resolves true when the server has it. */
  async function save(): Promise<boolean> {
    const typed = formRef.current
    const known = baseRef.current
    if (!typed || !known || busy || !changedGroups(typed, known).length) return false
    if (changedGroups(typed, known).includes('displayName')) {
      const text = nameProblemText(typed.displayName)
      if (text) {
        setNameError(text)
        nameRef.current?.focus()
        nameRef.current?.scrollIntoView?.({ block: 'center' })
        return false
      }
    }
    const textProblems = {
      ...(CONTROL.test(typed.jobTitle.trim()) ? { jobTitle: t('profile.textControl') } : {}),
      ...(CONTROL.test(typed.statusText.trim()) ? { statusText: t('profile.textControl') } : {}),
    }
    if (textProblems.jobTitle || textProblems.statusText) {
      setTextErrors(textProblems)
      const first = textProblems.jobTitle ? jobTitleRef : statusTextRef
      first.current?.focus()
      return false
    }
    setBusy(true)
    setFeedback(null)
    setError(null)
    try {
      // Just before saving, read the profile again: what changed elsewhere since this tab last heard is found now.
      let latest = known
      try {
        latest = valuesFromMe(await queryClient.fetchQuery({ ...queries.me(), staleTime: 0 }), fallbackZone)
      } catch {
        // Offline or the read failed: the save below says so.
      }
      const merged = mergeLatest(typed, known, latest, conflictsRef.current, kept.current)
      // What is typed while this runs stays: the newer server values go into the form as it is now, not as it was.
      setForm((current) => (current ? mergeLatest(current, known, latest, {}, kept.current).form : current))
      setBase(latest)
      setConflicts(merged.conflicts)
      if (Object.keys(merged.conflicts).some((group) => changedGroups(merged.form, latest).includes(group as Group))) {
        setFeedback({ kind: 'conflict' })
        return false
      }
      const payload = payloadFor(merged.form, latest)
      await api.updateProfile(payload)
      setBase(valuesAfterSave(payload, fallbackZone))
      setFeedback({ kind: 'saved', at: new Date() })
      void queryClient.invalidateQueries({ queryKey: queryKeys.me })
      return true
    } catch (caught) {
      // Everything else the server checks is checked above, so a refusal is the display name's.
      if (isApiError(caught) && caught.status === 400 && caught.code === 'validation') {
        setNameError(t('profile.nameRejected'))
        nameRef.current?.focus()
      } else {
        setFeedback({ kind: 'failed' })
      }
      return false
    } finally {
      setBusy(false)
    }
  }

  function onSubmit(event: FormEvent) {
    event.preventDefault()
    void save()
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
    setPhotoBusy(true)
    setError(null)
    try {
      await api.uploadAvatar(blob)
      setPhotoKey(Date.now())
      await queryClient.invalidateQueries({ queryKey: queryKeys.me })
    } catch (caught) {
      if (isApiError(caught)) setError(errorText(caught, t('profile.photoError')))
      else setError(t('profile.photoError'))
    } finally {
      setPhotoBusy(false)
    }
  }

  if (!me || !form || !base) return <p className="hint">{t('loading')}</p>
  const teamNames = people.data?.find((person) => person.userId === me.id)?.teams.map((team) => team.name) ?? []
  const crop = cropFile ? (
    <AvatarCropDialog file={cropFile} onCancel={() => setCropFile(null)} onConfirm={(blob) => void onCropped(blob)} />
  ) : null
  const statusLine = [form.statusEmoji, form.statusText].filter(Boolean).join(' ')
  const mark = (group: Group) => (changed.includes(group) ? t('profile.changed') : undefined)
  const conflictOf = (group: Group) => (open.includes(group) ? conflicts[group] : undefined)
  const note = (group: Group) => {
    const conflict = conflictOf(group)
    return conflict ? <ConflictNote group={group} conflict={conflict} onLatest={() => applyLatest(group)} onMine={() => keepMine(group)} /> : null
  }
  const showBar = dirty || feedback !== null
  return (
    <div className="page-scroll profile-page">
      {crop}
      <UnsavedBlocker active={dirty} onSave={save} copy={LEAVE_COPY} />
      <header className="page-intro">
        <h1>{t('settings.profile')}</h1>
        <p className="hint">{t('profile.lead')}</p>
      </header>
      <div className="profile-layout">
        <form className="card settings-card" noValidate onSubmit={onSubmit}>
          <div className="profile-photo">
            <Avatar name={form.displayName || me.displayName} userId={me.id} hasAvatar={me.hasAvatar} photoKey={photoKey || undefined} />
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
                    disabled={photoBusy}
                    onClick={() => {
                      setPhotoBusy(true)
                      void api.deleteAvatar().then(async () => {
                        setPhotoKey(Date.now())
                        await queryClient.invalidateQueries({ queryKey: queryKeys.me })
                      }).catch((caught: unknown) => setError(errorText(caught, t('error.generic'), t('error.dbUnavailable')))).finally(() => setPhotoBusy(false))
                    }}
                  >
                    {t('profile.remove')}
                  </Button>
                ) : null}
              </div>
            </div>
          </div>
          <Field label={t('profile.displayName')} hint={t('profile.displayNameHint')} badge={mark('displayName')}>
            <input
              ref={nameRef}
              value={form.displayName}
              maxLength={200}
              aria-invalid={nameError ? true : undefined}
              aria-describedby={nameError ? nameErrorId : undefined}
              onChange={(event) => edit({ displayName: event.target.value })}
            />
          </Field>
          {nameError ? <small id={nameErrorId} className="field-error" role="alert">{nameError}</small> : null}
          {note('displayName')}
          <Field label={t('profile.mention')} hint={t('profile.mentionHint')}>
            <input readOnly value={form.displayName.trim() ? `@${form.displayName.trim()}` : ''} />
          </Field>
          <Field label={t('profile.jobTitle')} badge={mark('jobTitle')}>
            <input
              ref={jobTitleRef}
              value={form.jobTitle}
              maxLength={80}
              aria-invalid={textErrors.jobTitle ? true : undefined}
              aria-describedby={textErrors.jobTitle ? jobTitleErrorId : undefined}
              onChange={(event) => edit({ jobTitle: event.target.value })}
            />
          </Field>
          {textErrors.jobTitle ? <small id={jobTitleErrorId} className="field-error" role="alert">{textErrors.jobTitle}</small> : null}
          {note('jobTitle')}
          <Field label={t('profile.timeZone')} hint={t('profile.timeZoneHint')} badge={mark('timeZone')}>
            <select value={form.timeZone} onChange={(event) => edit({ timeZone: event.target.value })}>
              {zones().map((zone) => <option key={zone} value={zone}>{zone}</option>)}
            </select>
          </Field>
          {note('timeZone')}
          <fieldset>
            <legend>{t('profile.status')}{mark('status') ? <em className="field-badge" aria-hidden="true">{mark('status')}</em> : null}</legend>
            <div className="row-actions">
              {PRESETS.map(([emoji, key]) => (
                <Button
                  key={key}
                  type="button"
                  variant="secondary"
                  onClick={() => edit({ statusEmoji: emoji, statusText: t(key) })}
                >
                  {emoji} {t(key)}
                </Button>
              ))}
              <Button type="button" variant="ghost" onClick={() => edit({ statusEmoji: '', statusText: '', statusClear: '' })}>
                {t('profile.statusNone')}
              </Button>
            </div>
            <Field label={t('profile.statusText')}>
              <input
                ref={statusTextRef}
                value={form.statusText}
                maxLength={80}
                aria-invalid={textErrors.statusText ? true : undefined}
                aria-describedby={textErrors.statusText ? statusTextErrorId : undefined}
                onChange={(event) => edit({ statusText: event.target.value })}
              />
            </Field>
            {textErrors.statusText ? <small id={statusTextErrorId} className="field-error" role="alert">{textErrors.statusText}</small> : null}
            <Field label={t('profile.statusClear')} hint={t('profile.statusClearHint')}>
              <input type="datetime-local" value={form.statusClear} onChange={(event) => edit({ statusClear: event.target.value })} />
            </Field>
            {note('status')}
          </fieldset>
          <p className="profile-email">
            <span>{t('account.email')}: {me.email}</span>
            <Link to={`/w/${workspaceId}/settings/account`}>{t('profile.emailHint')}</Link>
          </p>
          {error ? <p className="form-error" role="alert">{error}</p> : null}
          {showBar ? (
            <div className="profile-savebar" data-state={busy ? 'busy' : feedback?.kind ?? (dirty ? 'dirty' : 'idle')}>
              {feedback?.kind === 'failed' ? (
                <p className="savebar-problem" role="alert">
                  {t('profile.saveFailed')} <Button type="submit" variant="secondary" disabled={busy}>{t('profile.saveRetry')}</Button>
                </p>
              ) : null}
              {feedback?.kind === 'conflict' ? (
                <p className="savebar-problem" role="alert">
                  {t('profile.conflict.notSaved')} <Button type="submit" variant="secondary" disabled={busy}>{t('profile.saveRetry')}</Button>
                </p>
              ) : null}
              <div className="savebar-row">
                <span className="savebar-message" role="status">
                  {busy
                    ? t('profile.saving')
                    : dirty
                      ? t('profile.unsaved', { count: String(changed.length) })
                      : feedback?.kind === 'saved'
                        ? t('profile.savedAt', { time: new Intl.DateTimeFormat(locale, { timeStyle: 'short' }).format(feedback.at) })
                        : ''}
                </span>
                {dirty ? (
                  <span className="savebar-actions">
                    <Button type="button" variant="ghost" disabled={busy} onClick={revert}>{t('profile.revert')}</Button>
                    <Button type="submit" disabled={busy}>{t('account.save')}</Button>
                  </span>
                ) : null}
              </div>
            </div>
          ) : null}
        </form>
        <aside className="card settings-card profile-preview">
          <h2>{t('profile.preview')}{dirty ? <em className="field-badge preview-tag">{t('profile.beforeSave')}</em> : null}</h2>
          <div className="profile-preview-card">
            <Avatar name={form.displayName || me.displayName} userId={me.id} hasAvatar={me.hasAvatar} photoKey={photoKey || undefined} />
            <div>
              <strong>{form.displayName || me.displayName}</strong>
              <p className="hint">@{form.displayName.trim() || me.displayName}</p>
              {form.jobTitle.trim() ? <p>{form.jobTitle.trim()}</p> : null}
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

/** The item's two values side by side: what the person typed and what is saved now. Neither is applied until they choose. */
function ConflictNote({ group, conflict, onLatest, onMine }: { group: Group; conflict: Conflict; onLatest: () => void; onMine: () => void }) {
  const { t } = useI18n()
  const show = (values: ProfileValues): string => {
    const text = group === 'status'
      ? [[values.statusEmoji, values.statusText].filter(Boolean).join(' '), values.statusClear.replace('T', ' ')].filter(Boolean).join(' · ')
      : values[group].trim()
    return text || t('profile.conflict.empty')
  }
  return (
    <div className="field-conflict" role="group" aria-label={t('profile.conflict.title')}>
      <strong>{t('profile.conflict.title')}</strong>
      <dl>
        <div><dt>{t('profile.conflict.mine')}</dt><dd>{show(conflict.mine)}</dd></div>
        <div><dt>{t('profile.conflict.latest')}</dt><dd>{show(conflict.latest)}</dd></div>
      </dl>
      <span className="row-actions">
        <Button type="button" variant="secondary" onClick={onLatest}>{t('profile.conflict.useLatest')}</Button>
        <Button type="button" variant="ghost" onClick={onMine}>{t('profile.conflict.keepMine')}</Button>
      </span>
    </div>
  )
}
