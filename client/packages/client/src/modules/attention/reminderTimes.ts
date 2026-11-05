/** The times a person can pick for a Reminder, computed in their own clock. */
export type ReminderPreset = { id: 'in10' | 'later' | 'tomorrow' | 'nextWeek'; at: Date }

export function reminderPresets(now = new Date()): ReminderPreset[] {
  const presets: ReminderPreset[] = [{ id: 'in10', at: new Date(now.getTime() + 10 * 60_000) }]
  const evening = new Date(now)
  evening.setHours(18, 0, 0, 0)
  if (evening.getTime() - now.getTime() > 30 * 60_000) presets.push({ id: 'later', at: evening })
  const tomorrow = new Date(now)
  tomorrow.setDate(tomorrow.getDate() + 1)
  tomorrow.setHours(9, 0, 0, 0)
  presets.push({ id: 'tomorrow', at: tomorrow })
  const monday = new Date(now)
  const days = ((8 - monday.getDay()) % 7) || 7
  monday.setDate(monday.getDate() + days)
  monday.setHours(9, 0, 0, 0)
  presets.push({ id: 'nextWeek', at: monday })
  return presets
}

export function localTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
  } catch {
    return 'UTC'
  }
}

/** `datetime-local` text for a Date in the person's clock. */
export function toLocalInput(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}
