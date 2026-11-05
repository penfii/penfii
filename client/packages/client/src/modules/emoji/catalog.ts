export type EmojiEntry = {
  emoji: string
  name: string
}

export type EmojiCategory = {
  id: 'smileys' | 'gestures' | 'hearts' | 'objects'
  emojis: EmojiEntry[]
}

export const EMOJI_CATEGORIES: EmojiCategory[] = [
  {
    id: 'smileys',
    emojis: [
      { emoji: '😀', name: 'grin smile' },
      { emoji: '😁', name: 'beam smile' },
      { emoji: '😂', name: 'joy laugh' },
      { emoji: '🤣', name: 'rofl laugh' },
      { emoji: '😊', name: 'blush smile' },
      { emoji: '😍', name: 'heart eyes' },
      { emoji: '😘', name: 'kiss' },
      { emoji: '😎', name: 'cool' },
      { emoji: '🤔', name: 'think' },
      { emoji: '😅', name: 'sweat smile' },
      { emoji: '😢', name: 'cry' },
      { emoji: '😭', name: 'sob' },
      { emoji: '😡', name: 'angry' },
      { emoji: '😴', name: 'sleep' },
      { emoji: '🤗', name: 'hug' },
      { emoji: '😉', name: 'wink' },
    ],
  },
  {
    id: 'gestures',
    emojis: [
      { emoji: '👍', name: 'thumbs up' },
      { emoji: '👎', name: 'thumbs down' },
      { emoji: '👏', name: 'clap' },
      { emoji: '🙏', name: 'thanks pray' },
      { emoji: '👋', name: 'wave' },
      { emoji: '🤝', name: 'handshake' },
      { emoji: '✌️', name: 'peace' },
      { emoji: '🤞', name: 'fingers crossed' },
      { emoji: '💪', name: 'strong' },
      { emoji: '👀', name: 'eyes look' },
    ],
  },
  {
    id: 'hearts',
    emojis: [
      { emoji: '❤️', name: 'heart' },
      { emoji: '🧡', name: 'orange heart' },
      { emoji: '💛', name: 'yellow heart' },
      { emoji: '💚', name: 'green heart' },
      { emoji: '💙', name: 'blue heart' },
      { emoji: '💜', name: 'purple heart' },
      { emoji: '🖤', name: 'black heart' },
      { emoji: '💔', name: 'broken heart' },
      { emoji: '✨', name: 'sparkle' },
      { emoji: '🎉', name: 'party' },
      { emoji: '🔥', name: 'fire' },
      { emoji: '⭐', name: 'star' },
    ],
  },
  {
    id: 'objects',
    emojis: [
      { emoji: '✅', name: 'check done' },
      { emoji: '❌', name: 'cross no' },
      { emoji: '⚠️', name: 'warning' },
      { emoji: '📌', name: 'pin' },
      { emoji: '📎', name: 'paperclip file' },
      { emoji: '📝', name: 'memo note' },
      { emoji: '📅', name: 'calendar' },
      { emoji: '⏰', name: 'clock time' },
      { emoji: '💡', name: 'idea' },
      { emoji: '🚀', name: 'rocket' },
      { emoji: '🐛', name: 'bug' },
      { emoji: '💬', name: 'comment speech' },
    ],
  },
]

export function filterEmoji(query: string): EmojiEntry[] {
  const needle = query.trim().toLowerCase()
  const all = EMOJI_CATEGORIES.flatMap((category) => category.emojis)
  if (needle.length === 0) return all
  return all.filter((entry) => entry.name.includes(needle) || entry.emoji.includes(needle))
}
