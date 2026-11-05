import type { EstimateUnit, FieldChange, Ticket, TicketEstimate } from '@penfii/contracts'

/*
 * The planning values of a Ticket: a Task's estimate and its planned schedule. Pure helpers; the
 * server decides what is valid and the screen only helps a person write a value that will be.
 */

/** A decimal with at most two decimals and no sign, as the server reads it. */
export function isEstimateText(text: string): boolean {
  return /^\d{1,8}(\.\d{1,2})?$/.test(text.trim())
}

/** The shortest form of an estimate text: `5.00` is `5`, `0.50` is `0.5`. */
export function normalizeEstimateText(text: string): string {
  const [whole, fraction = ''] = text.trim().split('.')
  const kept = fraction.replace(/0+$/, '')
  return `${String(Number(whole))}${kept ? `.${kept}` : ''}`
}

/** An estimate as compared: hundredths, so no float is involved. */
export function estimateHundredths(text: string): number {
  const [whole, fraction = ''] = text.trim().split('.')
  return Number(whole) * 100 + Number(fraction.padEnd(2, '0'))
}

export function unitSuffix(unit: EstimateUnit | TicketEstimate['unit']): string {
  return unit === 'POINTS' ? 'pt' : unit === 'HOURS' ? 'h' : ''
}

/** `5 pt`, `0.5 h`. */
export function formatEstimate(estimate: TicketEstimate): string {
  return `${estimate.value} ${unitSuffix(estimate.unit)}`
}

/** Working minutes as written in the unit that suits them: `480` is 8 h, `90` is 1 h 30 min. */
export function splitMinutes(minutes: number): { hours: number; minutes: number } {
  return { hours: Math.floor(minutes / 60), minutes: minutes % 60 }
}

export function formatMinutes(minutes: number, labels: { hour: string; minute: string }): string {
  const { hours, minutes: rest } = splitMinutes(minutes)
  const parts: string[] = []
  if (hours > 0) parts.push(`${hours}${labels.hour}`)
  if (rest > 0 || hours === 0) parts.push(`${rest}${labels.minute}`)
  return parts.join(' ')
}

/** The estimate of a Ticket, or null when unestimated. */
export function estimateOf(ticket: Pick<Ticket, 'estimate'>): TicketEstimate | null {
  return ticket.estimate ?? null
}

/** An estimate as a number for sorting; unestimated sorts last in either direction. */
export function estimateSortValue(ticket: Pick<Ticket, 'estimate'>): number | null {
  const estimate = estimateOf(ticket)
  return estimate ? estimateHundredths(estimate.value) : null
}

/** The schedule parts of a revision change, empty string where a part is not set. */
export function scheduleParts(value: FieldChange['from']): { start: string; duration: string; milestone: boolean } | null {
  if (!Array.isArray(value) || value.length < 3) return null
  return { start: value[0] ?? '', duration: value[1] ?? '', milestone: value[2] === 'milestone' }
}

/** A schedule in the shape a revision change carries it, so a conflict row reads like a History row. */
export function scheduleAsChange(schedule: {
  plannedStartAt?: string | null
  durationMinutes?: number | null
  isScheduleMilestone?: boolean
} | null | undefined): string[] | null {
  if (!schedule) return null
  return [
    schedule.plannedStartAt ?? '',
    schedule.durationMinutes === null || schedule.durationMinutes === undefined ? '' : String(schedule.durationMinutes),
    schedule.isScheduleMilestone ? 'milestone' : '',
  ]
}

/** An estimate in the shape a revision change carries it. */
export function estimateAsChange(estimate: { value: string; unit?: string } | null | undefined, fallbackUnit: string): string[] | null {
  return estimate ? [estimate.value, estimate.unit ?? fallbackUnit] : null
}
