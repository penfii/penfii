import type { BulkResult } from '@penfii/contracts'
import { notify } from '../../design/notices'
import { useI18n } from '../../i18n/I18nProvider'
import { restoreArchived, type ArchivePlan } from './bulkLifecycle'

/** One line saying how many of the chosen Tickets an Archive leaves out, and why. Nothing when every one would go. */
export function ArchiveExcluded({ plan, count }: { plan: ArchivePlan; count: number }) {
  const { t } = useI18n()
  const reasons = [
    plan.openChildren > 0 ? t('bulk.lifecycle.reason.openChildren', { count: String(plan.openChildren) }) : '',
    plan.noPermission > 0 ? t('bulk.lifecycle.reason.noPermission', { count: String(plan.noPermission) }) : '',
    plan.unfinished > 0 ? t('bulk.lifecycle.reason.unfinished', { count: String(plan.unfinished) }) : '',
  ].filter(Boolean)
  if (count === 0 || reasons.length === 0) return null
  return <p className="bulk-excluded" role="note">{t('bulk.lifecycle.excluded', { count: String(count - plan.ids.length), reasons: reasons.join(', ') })}</p>
}

/**
 * After an Archive in which every Ticket went: a notice with Undo, which takes them all out of the archive again. A partial
 * run has no Undo here; its result stays on screen with the reasons instead. `onUndone` runs once the Undo has finished.
 */
export function announceArchived(t: ReturnType<typeof useI18n>['t'], outcome: BulkResult, onUndone: (restored: number) => void) {
  const archived = outcome.items.filter((item) => item.ok).map((item) => item.ticketId)
  if (archived.length === 0 || outcome.failed > 0) return
  notify({
    message: t('bulk.lifecycle.archiveDone', { total: String(outcome.total) }),
    action: {
      label: t('notice.undo'),
      run: () => void restoreArchived(archived).then(({ ok, failed }) => {
        onUndone(ok)
        if (failed === 0) notify({ message: t('bulk.lifecycle.unarchived', { count: String(ok) }) })
        else notify({ kind: 'error', message: t('bulk.lifecycle.unarchivePartial', { total: String(archived.length), ok: String(ok), failed: String(failed) }) })
      }),
    },
  })
}
