import { useRef, useState } from 'react'
import type { InboxItem, InboxType } from '@penfii/contracts'
import { Button, useDismiss } from '../../design/ui'
import { ArchiveIcon, CheckIcon, ClockIcon, DotIcon, SunriseIcon, UndoIcon } from '../../design/icons'
import { formatMessageTime } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { INBOX_TYPES, VISIBLE_TYPE_CHIPS, groupByDay, toggleType, type InboxLocation, type InboxViewOptions } from './inboxModel'

/** The Filter chips: `읽지 않음`, the types, and "More" for the types past the first few. Used by the Inbox and the archive. */
export function InboxChips({ location, onChange, showUnread }: { location: InboxLocation; onChange: (next: InboxLocation) => void; showUnread: boolean }) {
  const { t } = useI18n()
  const hidden = INBOX_TYPES.slice(VISIBLE_TYPE_CHIPS)
  const [openMore, setOpenMore] = useState(false)
  const expanded = openMore || hidden.some((type) => location.types.includes(type))
  const shown = expanded ? INBOX_TYPES : INBOX_TYPES.slice(0, VISIBLE_TYPE_CHIPS)
  function chip(type: InboxType) {
    const on = location.types.includes(type)
    return (
      <button key={type} type="button" className={on ? 'chip on' : 'chip'} aria-pressed={on} onClick={() => onChange(toggleType(location, type))}>
        {t(`inbox.filter.${type}`)}
      </button>
    )
  }
  return (
    <div className="inbox-chips" role="group" aria-label={t('inbox.filters')}>
      {showUnread ? (
        <>
          <button type="button" className={location.unread ? 'chip on' : 'chip'} aria-pressed={location.unread} onClick={() => onChange({ ...location, unread: !location.unread })}>
            {t('inbox.unreadOnly')}
          </button>
          <span className="chip-divider" aria-hidden="true" />
        </>
      ) : null}
      {shown.map(chip)}
      {hidden.length > 0 && !expanded ? (
        <button type="button" className="chip more" aria-expanded={false} onClick={() => setOpenMore(true)}>{t('inbox.filterMore')}</button>
      ) : null}
    </div>
  )
}

/** The View Options popover: local to this device. The archive has no read or snooze option, only grouping. */
export function InboxViewOptionsMenu({ options, onChange, archive, unreadFilter }: {
  options: InboxViewOptions
  onChange: (patch: Partial<InboxViewOptions>) => void
  archive: boolean
  unreadFilter: boolean
}) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  useDismiss(open, () => setOpen(false), ref)
  return (
    <div className="inbox-options" ref={ref} onKeyDown={(event) => { if (event.key === 'Escape') setOpen(false) }}>
      <Button type="button" variant="secondary" aria-haspopup="true" aria-expanded={open} onClick={() => setOpen((v) => !v)}>{t('inbox.options')}</Button>
      {open ? (
        <div className="inbox-options-panel" role="group" aria-label={t('inbox.options')}>
          {archive ? null : (
            <>
              <label className="inbox-option">
                <input type="checkbox" checked={options.showRead || unreadFilter} disabled={unreadFilter} onChange={(e) => onChange({ showRead: e.target.checked })} />
                <span>{t('inbox.options.showRead')}</span>
              </label>
              {unreadFilter ? <small className="hint">{t('inbox.options.unreadWins')}</small> : null}
              <label className="inbox-option">
                <input type="checkbox" checked={options.showSnoozed} onChange={(e) => onChange({ showSnoozed: e.target.checked })} />
                <span>{t('inbox.options.showSnoozed')}</span>
              </label>
            </>
          )}
          <label className="inbox-option">
            <input type="checkbox" checked={options.groupByDay} onChange={(e) => onChange({ groupByDay: e.target.checked })} />
            <span>{t('inbox.options.groupByDay')}</span>
          </label>
        </div>
      ) : null}
    </div>
  )
}

export type InboxRowActions = {
  open: (item: InboxItem) => void
  run: (item: InboxItem, command: 'read' | 'unread' | 'archive' | 'unarchive' | 'snooze', until?: string) => void
}

function tomorrowMorning(): string {
  const next = new Date()
  next.setDate(next.getDate() + 1)
  next.setHours(9, 0, 0, 0)
  return next.toISOString()
}

function isSnoozed(item: InboxItem): boolean {
  return item.snoozedUntil !== null && new Date(item.snoozedUntil).getTime() > Date.now()
}

function InboxRow({ item, archive, actions }: { item: InboxItem; archive: boolean; actions: InboxRowActions }) {
  const { t, locale } = useI18n()
  const late = typeof item.params.lateSeconds === 'number' ? Math.round(item.params.lateSeconds / 60) : 0
  const snoozed = !archive && isSnoozed(item)
  const stamp = archive ? (item.archivedAt ?? item.createdAt) : item.createdAt
  return (
    <li className={item.state === 'UNREAD' && !archive ? 'result-hit inbox-row unread' : 'result-hit inbox-row'}>
      <button type="button" className="inbox-open" onClick={() => actions.open(item)}>
        <span className="inbox-text">
          {item.state === 'UNREAD' && !archive ? <span className="unread-dot" role="img" aria-label={t('inbox.unreadDot')} /> : null}
          {t(`inbox.kind.${item.kind}`, { actor: item.actor?.name ?? t('inbox.someone') })}
        </span>
        <strong className="inbox-subject">{item.subject.title}</strong>
        <small className="hint">
          {archive ? `${t('inbox.archivedAt')} ` : ''}
          {formatMessageTime(stamp, locale)}
          {late > 0 ? ` · ${t('inbox.late', { minutes: String(late) })}` : ''}
          {snoozed && item.snoozedUntil ? ` · ${t('inbox.snoozedUntil', { time: formatMessageTime(item.snoozedUntil, locale) })}` : ''}
        </small>
      </button>
      <span className="saved-actions">
        {archive ? (
          <Button type="button" variant="secondary" onClick={() => actions.run(item, 'unarchive')}>{t('inbox.restore')}</Button>
        ) : (
          <>
            {snoozed ? (
              <button type="button" className="icon-button" title={t('inbox.unsnooze')} aria-label={t('inbox.unsnooze')} onClick={() => actions.run(item, 'unread')}>
                <UndoIcon />
              </button>
            ) : (
              <button type="button" className="icon-button" title={item.state === 'UNREAD' ? t('inbox.markRead') : t('inbox.markUnread')} aria-label={item.state === 'UNREAD' ? t('inbox.markRead') : t('inbox.markUnread')} onClick={() => actions.run(item, item.state === 'UNREAD' ? 'read' : 'unread')}>{item.state === 'UNREAD' ? <CheckIcon /> : <DotIcon />}</button>
            )}
            <button type="button" className="icon-button" title={t('inbox.snooze1h')} aria-label={`${t('inbox.snooze')}: ${t('inbox.snooze1h')}`} onClick={() => actions.run(item, 'snooze', new Date(Date.now() + 3_600_000).toISOString())}>
              <ClockIcon />
            </button>
            <button type="button" className="icon-button" title={t('inbox.snoozeTomorrow')} aria-label={`${t('inbox.snooze')}: ${t('inbox.snoozeTomorrow')}`} onClick={() => actions.run(item, 'snooze', tomorrowMorning())}>
              <SunriseIcon />
            </button>
            <button type="button" className="icon-button" title={t('inbox.archive')} aria-label={t('inbox.archive')} onClick={() => actions.run(item, 'archive')}>
              <ArchiveIcon />
            </button>
          </>
        )}
      </span>
    </li>
  )
}

/** The list, optionally grouped by day (Today / Yesterday / date). Shared by the Inbox and the archive; they differ in the date and the row actions. */
export function InboxRows({ items, archive, groupByDay: group, actions }: { items: InboxItem[]; archive: boolean; groupByDay: boolean; actions: InboxRowActions }) {
  const { t, locale } = useI18n()
  if (!group) {
    return (
      <ul className="result-list inbox-list">
        {items.map((item) => <InboxRow key={item.id} item={item} archive={archive} actions={actions} />)}
      </ul>
    )
  }
  const groups = groupByDay(items, (item) => (archive ? (item.archivedAt ?? item.createdAt) : item.createdAt))
  return (
    <div className="inbox-groups">
      {groups.map((day) => (
        <section key={day.key} aria-label={day.day === 'date' ? day.date : t(`inbox.day.${day.day}`)}>
          <h2 className="inbox-day">
            {day.day === 'date' ? new Intl.DateTimeFormat(locale, { dateStyle: 'medium' }).format(new Date(day.date)) : t(`inbox.day.${day.day}`)}
          </h2>
          <ul className="result-list inbox-list">
            {day.items.map((item) => <InboxRow key={item.id} item={item} archive={archive} actions={actions} />)}
          </ul>
        </section>
      ))}
    </div>
  )
}
