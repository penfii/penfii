import type { RichTextDoc } from '@penfii/contracts'
import { useI18n } from '../../i18n/I18nProvider'
import { plainText } from '../../richtext'

export function MessageReference({
  name,
  message,
  onOpen,
}: {
  name: string
  message: { deleted: boolean; body: RichTextDoc | null } | null
  onOpen?: () => void
}) {
  const { t } = useI18n()
  const doc = message?.body
  const missing = !message || message.deleted || doc == null
  const text = doc == null || message?.deleted ? t('message.deleted') : plainText(doc).replace(/\s+/g, ' ').trim()
  const short = text.length > 80 ? `${text.slice(0, 80)}…` : text
  const body = (
    <>
      <span className="reference-name">↳ {missing ? t('message.deleted') : name}</span>
      {missing ? null : <span className="reference-text">{short}</span>}
    </>
  )
  if (!onOpen) return <div className={missing ? 'reference missing' : 'reference'}>{body}</div>
  return (
    <button type="button" className={missing ? 'reference missing' : 'reference'} onClick={onOpen}>
      {body}
    </button>
  )
}
