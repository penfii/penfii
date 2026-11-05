import { RichTextView, type DisplayNames } from '../../richtext'
import { useI18n } from '../../i18n/I18nProvider'
import type { OutboundAttempt } from './useOutboundMessage'
import { Button } from '../../design/ui'

export function OutgoingRow({
  attempt,
  userId,
  names,
  onRetry,
}: {
  attempt: OutboundAttempt
  userId: string
  names: DisplayNames
  onRetry: () => void
}) {
  const { t } = useI18n()
  return (
    <article
      className={attempt.phase === 'sending' ? 'message outgoing-sending' : 'message outgoing-failed'}
      data-send-phase={attempt.phase}
    >
      <header className="message-meta">
        <strong>{names[userId] ?? userId}</strong>
        <span>{attempt.phase === 'sending' ? t('message.sending') : t('message.failed')}</span>
      </header>
      <RichTextView doc={attempt.doc} names={names} />
      {attempt.phase === 'failed' ? (
        <Button type="button" variant="secondary" onClick={onRetry}>
          {t('message.retry')}
        </Button>
      ) : null}
    </article>
  )
}
