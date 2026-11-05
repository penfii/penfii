/*
 * A planned start is an instant (UTC) that a person reads and types as a wall-clock time in the
 * Project calendar's zone. The browser only converts for the zone it runs in, so the conversion to
 * and from another IANA zone is done with Intl, the same zone rules the server takes from its
 * database. No date library is involved.
 */

/** The zone this browser runs in. */
export function browserZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
}

const formatters = new Map<string, Intl.DateTimeFormat>()

function formatterFor(timeZone: string): Intl.DateTimeFormat {
  let formatter = formatters.get(timeZone)
  if (!formatter) {
    formatter = new Intl.DateTimeFormat('en-US', {
      timeZone,
      hourCycle: 'h23',
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    })
    formatters.set(timeZone, formatter)
  }
  return formatter
}

type Wall = { year: number; month: number; day: number; hour: number; minute: number; second: number }

function wallClock(instant: Date, timeZone: string): Wall {
  const parts = formatterFor(timeZone).formatToParts(instant)
  const value = (type: string) => Number(parts.find((part) => part.type === type)?.value ?? '0')
  return { year: value('year'), month: value('month'), day: value('day'), hour: value('hour'), minute: value('minute'), second: value('second') }
}

/** Minutes the zone is ahead of UTC at this instant. */
export function zoneOffsetMinutes(instant: Date, timeZone: string): number {
  const wall = wallClock(instant, timeZone)
  const asUtc = Date.UTC(wall.year, wall.month - 1, wall.day, wall.hour, wall.minute, wall.second)
  return Math.round((asUtc - Math.floor(instant.getTime() / 1000) * 1000) / 60000)
}

const pad = (value: number) => String(value).padStart(2, '0')

/** An instant as `YYYY-MM-DDTHH:mm` on the wall clock of `timeZone`, the value of a datetime-local input. */
export function instantToLocalInput(iso: string, timeZone: string): string {
  const wall = wallClock(new Date(iso), timeZone)
  return `${String(wall.year).padStart(4, '0')}-${pad(wall.month)}-${pad(wall.day)}T${pad(wall.hour)}:${pad(wall.minute)}`
}

/**
 * The instant at which the wall clock of `timeZone` reads `local` (`YYYY-MM-DDTHH:mm`), as an ISO
 * string in UTC. A time the clocks skip is read as the same wall time after the change; a time that
 * happens twice is read as its first occurrence. `null` when `local` is not a date and time.
 */
export function localInputToInstant(local: string, timeZone: string): string | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/.exec(local)
  if (!match) return null
  const [year, month, day, hour, minute] = match.slice(1).map(Number)
  const asUtc = Date.UTC(year, month - 1, day, hour, minute)
  const check = new Date(asUtc)
  if (Number.isNaN(asUtc) || check.getUTCMonth() !== month - 1 || check.getUTCDate() !== day || check.getUTCHours() !== hour || check.getUTCMinutes() !== minute) return null
  // The offset depends on the answer, so settle it from both sides of a change.
  const first = asUtc - zoneOffsetMinutes(new Date(asUtc), timeZone) * 60000
  const second = asUtc - zoneOffsetMinutes(new Date(first), timeZone) * 60000
  const candidates = [first, second].filter((candidate) => instantToLocalInput(new Date(candidate).toISOString(), timeZone) === local)
  const chosen = candidates.length > 0 ? Math.min(...candidates) : Math.max(first, second)
  return new Date(chosen).toISOString().replace('.000Z', 'Z')
}

/** An instant as a person reads it in `timeZone`: date and time, in their language. */
export function formatZoned(iso: string, timeZone: string, locale: string): string {
  return new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short', timeZone }).format(new Date(iso))
}

/** The IANA zones this browser knows, for a list to pick from; empty where the browser cannot say. */
export function timeZones(): string[] {
  try {
    const intl = Intl as unknown as { supportedValuesOf?: (key: string) => string[] }
    return intl.supportedValuesOf?.('timeZone') ?? []
  } catch {
    return []
  }
}
