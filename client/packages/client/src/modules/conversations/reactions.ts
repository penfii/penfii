import type { Message, Reaction } from '@penfii/contracts'

export const REACTION_EMOJI = ['👍', '❤️', '😂', '🎉', '👀'] as const

export function reactionPressed(reaction: Reaction | undefined, userId: string): boolean {
  if (!reaction) return false
  if (reaction.userIds.length > 0) return reaction.userIds.includes(userId)
  return reaction.reactedByMe
}

export function reactionCount(reaction: Reaction | undefined): number {
  if (!reaction) return 0
  if (reaction.userIds.length > 0) return reaction.userIds.length
  return reaction.count
}

export function withToggledReaction(message: Message, emoji: string, userId: string): Message {
  const reactions = message.reactions.map((reaction) => ({
    ...reaction,
    userIds: [...reaction.userIds],
  }))
  const index = reactions.findIndex((reaction) => reaction.emoji === emoji)
  if (index < 0) {
    reactions.push({ emoji, count: 1, reactedByMe: true, userIds: [userId] })
    return { ...message, reactions }
  }
  const current = reactions[index]
  if (!current) return message
  if (reactionPressed(current, userId)) {
    const userIds = current.userIds.filter((id) => id !== userId)
    const count = Math.max(0, (current.userIds.length > 0 ? current.userIds.length : current.count) - 1)
    if (count <= 0) reactions.splice(index, 1)
    else reactions[index] = { emoji, count, reactedByMe: false, userIds }
  } else {
    const userIds = current.userIds.includes(userId) ? current.userIds : [...current.userIds, userId]
    const count = (current.userIds.length > 0 ? current.userIds.length : current.count) + 1
    reactions[index] = { emoji, count, reactedByMe: true, userIds }
  }
  return { ...message, reactions }
}
