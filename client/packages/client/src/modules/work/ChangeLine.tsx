import type { FieldChange, TicketField } from '@penfii/contracts'
import { useI18n } from '../../i18n/I18nProvider'
import { formatStamp } from '../../format'
import { formatMinutes, scheduleParts, unitSuffix } from './planValues'
import { formatDue, isPriority } from './ticketModel'

type Translate = ReturnType<typeof useI18n>['t']

export function fieldLabelKey(field: TicketField) {
  return `work.field.${field}` as const
}

/** A value as a person reads it: priorities and dates in their language, empty as “None”. */
export function formatChangeValue(field: TicketField, value: FieldChange['from'], t: Translate, locale: string): string {
  if (value === null || (Array.isArray(value) && value.length === 0)) return t('work.value.empty')
  if (field === 'estimate' && Array.isArray(value)) return `${value[0] ?? ''} ${unitSuffix(value[1] === 'HOURS' ? 'HOURS' : 'POINTS')}`.trim()
  if (field === 'schedule') {
    const parts = scheduleParts(value)
    if (!parts) return t('work.value.empty')
    const labels = { hour: t('work.schedule.hourShort'), minute: t('work.schedule.minuteShort') }
    const shown = [
      parts.start ? `${t('work.schedule.start')} ${formatStamp(parts.start, locale)}` : '',
      parts.milestone ? t('work.schedule.milestoneChip') : parts.duration ? t('work.schedule.summary', { duration: formatMinutes(Number(parts.duration), labels) }) : '',
    ].filter(Boolean)
    return shown.length > 0 ? shown.join(' · ') : t('work.value.empty')
  }
  const text = Array.isArray(value) ? value.join(', ') : value
  if (field === 'priority' && isPriority(text)) return t(`work.priority.${text}`)
  if (field === 'dueOn') return formatDue(text, locale)
  return text
}

/** One changed field: its name, the value before and the value after. */
export function ChangeLine({
  change,
  leftLabel,
  rightLabel,
}: {
  change: FieldChange
  leftLabel?: string
  rightLabel?: string
}) {
  const { t, locale } = useI18n()
  const wide = change.field === 'description' || change.field === 'title'
  const from = formatChangeValue(change.field, change.from, t, locale)
  const to = formatChangeValue(change.field, change.to, t, locale)
  return (
    <div className={wide ? 'change-line wide' : 'change-line'}>
      <span className="change-field">{t(fieldLabelKey(change.field))}</span>
      <span className={change.from === null ? 'change-value empty' : 'change-value'} aria-label={leftLabel}>{from}</span>
      <span className="change-arrow" aria-hidden="true">→</span>
      <span className={change.to === null ? 'change-value empty' : 'change-value'} aria-label={rightLabel}>{to}</span>
    </div>
  )
}
