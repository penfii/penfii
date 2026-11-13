import { useI18n } from '../i18n/I18nProvider'
import type { MentionCandidate } from './editorExtensions'

export type MentionUi = {
  items: MentionCandidate[]
  selected: number
  loading: boolean
  rect: DOMRect | null
  choose: (candidate: MentionCandidate) => void
}

export const MENTION_LIST_ID = 'mention-suggestions'
export const mentionOptionId = (index: number) => `${MENTION_LIST_ID}-${String(index)}`

/**
 * The people matching what follows `@`. It says nothing about who will be notified: a person outside
 * the channel is only drawn dimmer, and the server decides who is told.
 */
export function MentionList({ ui }: { ui: MentionUi }) {
  const { t } = useI18n()
  const rect = ui.rect
  const below = !rect || rect.top < 260
  const style = rect
    ? below
      ? { left: Math.max(8, rect.left), top: rect.bottom + 4 }
      : { left: Math.max(8, rect.left), bottom: window.innerHeight - rect.top + 4 }
    : { left: 8, bottom: 8 }
  return (
    <>
      <div className="mention-list" style={{ position: 'fixed', ...style }}>
        {ui.items.length === 0 ? (
          <p className="mention-empty" role="status">{ui.loading ? t('mention.loading') : t('mention.empty')}</p>
        ) : (
          <ul id={MENTION_LIST_ID} role="listbox" aria-label={t('mention.suggestions')} onMouseDown={(event) => event.preventDefault()}>
            {ui.items.map((candidate, index) => (
              <li
                key={candidate.id}
                id={mentionOptionId(index)}
                role="option"
                aria-selected={index === ui.selected}
                className={['mention-option', index === ui.selected ? 'is-selected' : '', candidate.member ? '' : 'is-nonmember'].filter(Boolean).join(' ')}
                onClick={() => ui.choose(candidate)}
              >
                <span className="mention-name">{candidate.name}</span>
                {candidate.detail ? <small className="mention-detail">{candidate.detail}</small> : null}
                {candidate.member ? null : <span className="visually-hidden">{t('mention.nonMember')}</span>}
              </li>
            ))}
          </ul>
        )}
      </div>
    </>
  )
}
