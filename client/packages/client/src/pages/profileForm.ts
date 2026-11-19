import type { Me } from '@penfii/contracts'

/**
 * The profile form as plain values, and the rules that keep what the person typed when the profile changes under
 * them. A field the person has not touched follows the server; one they have touched is theirs, and when the server
 * changed it too the two are shown side by side instead of one silently replacing the other.
 */
export type ProfileValues = {
  displayName: string
  jobTitle: string
  timeZone: string
  statusEmoji: string
  statusText: string
  /** `datetime-local` text, or empty. */
  statusClear: string
}

/** What is compared, counted and merged: the status is one item, whatever mix of its three fields changed. */
export type Group = 'displayName' | 'jobTitle' | 'timeZone' | 'status'
export const GROUPS: readonly Group[] = ['displayName', 'jobTitle', 'timeZone', 'status']
const MEMBERS: Record<Group, ReadonlyArray<keyof ProfileValues>> = {
  displayName: ['displayName'],
  jobTitle: ['jobTitle'],
  timeZone: ['timeZone'],
  status: ['statusEmoji', 'statusText', 'statusClear'],
}

export function toLocalInput(iso: string): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return ''
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${String(date.getFullYear())}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}

export function valuesFromMe(me: Me, fallbackZone: string): ProfileValues {
  return {
    displayName: me.displayName,
    jobTitle: me.jobTitle ?? '',
    timeZone: me.timeZone ?? fallbackZone,
    statusEmoji: me.statusEmoji ?? '',
    statusText: me.statusText ?? '',
    statusClear: me.statusClearAt ? toLocalInput(me.statusClearAt) : '',
  }
}

/** Surrounding spaces are not a difference: they are not saved. */
export function cleaned(values: ProfileValues): ProfileValues {
  return { ...values, displayName: values.displayName.trim(), jobTitle: values.jobTitle.trim(), statusEmoji: values.statusEmoji.trim(), statusText: values.statusText.trim() }
}

/** One item of the form as comparable text (surrounding spaces ignored). */
export function groupSlice(values: ProfileValues, group: Group): string {
  const clean = cleaned(values)
  return JSON.stringify(MEMBERS[group].map((key) => clean[key]))
}

export function changedGroups(form: ProfileValues, base: ProfileValues): Group[] {
  return GROUPS.filter((group) => groupSlice(form, group) !== groupSlice(base, group))
}

function copyGroup(into: ProfileValues, from: ProfileValues, group: Group): ProfileValues {
  const next = { ...into }
  for (const key of MEMBERS[group]) next[key] = from[key]
  return next
}

/** Puts one item back to the server's value. */
export function takeLatest(form: ProfileValues, base: ProfileValues, group: Group): ProfileValues {
  return copyGroup(form, base, group)
}

export type Conflict = { mine: ProfileValues; latest: ProfileValues }
export type Conflicts = Partial<Record<Group, Conflict>>

/**
 * The profile as the server now has it (`next`), after it was `previous`. Items the person did not touch take the new value; items they
 * did touch stay, and where the server's value also moved to something else than what they typed, a conflict is noted.
 * `kept` holds the server values a person already answered "keep mine" to, so the same value does not ask again.
 */
export function mergeLatest(form: ProfileValues, previous: ProfileValues, next: ProfileValues, conflicts: Conflicts, kept: Partial<Record<Group, string>>): { form: ProfileValues; conflicts: Conflicts } {
  let merged = form
  const found: Conflicts = {}
  for (const group of GROUPS) {
    const mine = groupSlice(form, group)
    const was = groupSlice(previous, group)
    const now = groupSlice(next, group)
    if (mine === was) merged = copyGroup(merged, next, group)
    else if (now === mine) continue
    else if (now !== was && kept[group] !== now) found[group] = { mine: form, latest: next }
    else if (conflicts[group] && kept[group] !== now) found[group] = { mine: form, latest: next }
  }
  return { form: merged, conflicts: found }
}

/** What goes to the server: the person's value for items they changed, the latest known value for the rest. */
export function payloadFor(form: ProfileValues, base: ProfileValues) {
  const changed = new Set(changedGroups(form, base))
  const pick = (group: Group) => cleaned(changed.has(group) ? form : base)
  const name = pick('displayName')
  const title = pick('jobTitle')
  const zone = pick('timeZone')
  const status = pick('status')
  return {
    displayName: name.displayName,
    jobTitle: title.jobTitle || null,
    timeZone: zone.timeZone || null,
    statusEmoji: status.statusEmoji || null,
    statusText: status.statusText || null,
    statusClearAt: status.statusClear ? new Date(status.statusClear).toISOString() : null,
  }
}

/** The values the server holds after a save of `payload`, in form shape. */
export function valuesAfterSave(payload: ReturnType<typeof payloadFor>, fallbackZone: string): ProfileValues {
  return {
    displayName: payload.displayName,
    jobTitle: payload.jobTitle ?? '',
    timeZone: payload.timeZone ?? fallbackZone,
    statusEmoji: payload.statusEmoji ?? '',
    statusText: payload.statusText ?? '',
    statusClear: payload.statusClearAt ? toLocalInput(payload.statusClearAt) : '',
  }
}
