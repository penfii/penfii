import { useMemo, useState } from 'react'
import { useI18n } from '../../i18n/I18nProvider'
import type { MessageKey } from '../../i18n/messages'
import { EMOJI_CATEGORIES, filterEmoji, type EmojiCategory } from './catalog'

const CATEGORY_LABEL: Record<EmojiCategory['id'], MessageKey> = {
  smileys: 'composer.emoji.smileys',
  gestures: 'composer.emoji.gestures',
  hearts: 'composer.emoji.hearts',
  objects: 'composer.emoji.objects',
}

const RECENT_KEY = 'penfii.recentEmoji'

function readRecent(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_KEY)
    const parsed = raw ? JSON.parse(raw) as unknown : []
    return Array.isArray(parsed) ? parsed.filter((item): item is string => typeof item === 'string').slice(0, 16) : []
  } catch {
    return []
  }
}

function remember(emoji: string) {
  const next = [emoji, ...readRecent().filter((item) => item !== emoji)].slice(0, 16)
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(next))
  } catch {
    // Recent emoji is a local convenience. The inserted character is already in the message.
  }
}

export function EmojiPicker({ onPick }: { onPick: (emoji: string) => void }) {
  const { t } = useI18n()
  const [query, setQuery] = useState('')
  const [category, setCategory] = useState<EmojiCategory['id'] | 'recent'>('smileys')
  const [recent, setRecent] = useState(readRecent)
  const searching = query.trim().length > 0
  const entries = useMemo(() => {
    if (searching) return filterEmoji(query)
    if (category === 'recent') return recent.map((emoji) => ({ emoji, name: emoji }))
    return EMOJI_CATEGORIES.find((item) => item.id === category)?.emojis ?? []
  }, [category, query, recent, searching])

  return (
    <div className="emoji-picker" role="dialog" aria-label={t('composer.emoji')}>
      <input
        aria-label={t('composer.emojiSearch')}
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder={t('composer.emojiSearch')}
      />
      {searching ? null : (
        <div className="emoji-categories" role="tablist">
          <button type="button" aria-pressed={category === 'recent'} onClick={() => setCategory('recent')}>{t('composer.emojiRecent')}</button>
          {EMOJI_CATEGORIES.map((item) => (
            <button key={item.id} type="button" aria-pressed={category === item.id} onClick={() => setCategory(item.id)}>
              {t(CATEGORY_LABEL[item.id])}
            </button>
          ))}
        </div>
      )}
      <div className="emoji-grid">
        {entries.map((entry) => (
          <button
            key={entry.emoji}
            type="button"
            aria-label={entry.name}
            onClick={() => {
              remember(entry.emoji)
              setRecent(readRecent())
              onPick(entry.emoji)
            }}
          >
            {entry.emoji}
          </button>
        ))}
      </div>
    </div>
  )
}
