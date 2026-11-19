import type { BulkResult } from '@penfii/contracts'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { BulkResultView } from './BulkBar'
import type { ArchivePlan, LifecycleAction } from './bulkLifecycle'
import { ArchiveExcluded } from './archiveFeedback'

/**
 * Archive or trash for the Completed/Canceled cards chosen on the Board. The chosen set is one
 * operation: the result names every Ticket, and a partial run is never shown as a success; the Tickets
 * that were not processed stay chosen so the same action can be tried again.
 */
export function BulkLifecycleBar({
  count,
  plan,
  scope,
  running,
  result,
  last,
  names,
  onRun,
  onClear,
}: {
  count: number
  /** How many of the chosen an Archive would take, and why the rest stay out. */
  plan: ArchivePlan
  /** One line on what the choice covers (the loaded, filtered Tickets). */
  scope: string
  running: { done: number; total: number } | null
  result: BulkResult | null
  last: LifecycleAction | null
  names: Map<string, { key: string; title: string }>
  onRun: (action: LifecycleAction) => void
  onClear: () => void
}) {
  const { t } = useI18n()
  const failed = result !== null && result.failed > 0
  return (
    <div className="bulk-bar" role="region" aria-label={t('bulk.title')}>
      <div className="bulk-controls">
        {count > 0 ? (
          <>
            <strong>{t('bulk.lifecycle.selected', { count: String(count) })}</strong>
            <Button type="button" disabled={running !== null || plan.ids.length === 0} onClick={() => onRun('ARCHIVE')}>{t('bulk.lifecycle.archiveCount', { count: String(plan.ids.length) })}</Button>
            <Button type="button" variant="secondary" disabled={running !== null} onClick={() => onRun('TRASH')}>{t('bulk.lifecycle.trash')}</Button>
            {failed && last ? <Button type="button" variant="secondary" disabled={running !== null} onClick={() => onRun(last)}>{t('bulk.lifecycle.retry')}</Button> : null}
          </>
        ) : null}
        <Button type="button" variant="ghost" disabled={running !== null} onClick={onClear}>{t(count > 0 ? 'bulk.lifecycle.clear' : 'bulk.lifecycle.dismiss')}</Button>
      </div>
      <ArchiveExcluded plan={plan} count={count} />
      {count > 0 ? <p className="bulk-scope">{scope}</p> : null}
      {running ? <p className="hint" role="status">{t('bulk.lifecycle.running', { done: String(running.done), total: String(running.total) })}</p> : null}
      {result ? (
        <p className={failed ? 'board-error' : 'hint'} role={failed ? 'alert' : 'status'}>
          {failed
            ? t(last === 'TRASH' ? 'bulk.lifecycle.trashPartial' : last === 'ARCHIVE' ? 'bulk.lifecycle.archivePartial' : 'bulk.lifecycle.partial', { total: String(result.total), ok: String(result.succeeded), failed: String(result.failed) })
            : t(last === 'TRASH' ? 'bulk.lifecycle.trashDone' : last === 'ARCHIVE' ? 'bulk.lifecycle.archiveDone' : 'bulk.lifecycle.allDone', { total: String(result.total) })}
        </p>
      ) : null}
      {failed ? <BulkResultView result={result} names={names} /> : null}
    </div>
  )
}
