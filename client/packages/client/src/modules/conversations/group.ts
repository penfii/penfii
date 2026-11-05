/** Local clock minute shown on a message, `HH:mm`. */
export function displayedMinute(iso: string): string | null {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return null
  const hours = String(date.getHours()).padStart(2, '0')
  const minutes = String(date.getMinutes()).padStart(2, '0')
  return `${hours}:${minutes}`
}

function minuteKey(iso: string): string | null {
  const date = new Date(iso)
  const minute = displayedMinute(iso)
  if (!minute || Number.isNaN(date.getTime())) return null
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()} ${minute}`
}

/** Same sender and the same displayed minute stay in one group. */
export function continuesGroup(
  previous: { authorId: string; createdAt: string } | undefined,
  current: { authorId: string; createdAt: string },
): boolean {
  if (!previous || previous.authorId !== current.authorId) return false
  const earlier = minuteKey(previous.createdAt)
  const later = minuteKey(current.createdAt)
  return earlier != null && earlier === later
}
