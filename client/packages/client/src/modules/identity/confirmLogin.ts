import { api } from '../../api'
import { setLastMethod, takeProviderAttempt } from './loginMemory'

/**
 * After a provider round trip: remember that provider as the last method, but only when the session that is
 * signed in now says it was made by that provider, after the button was pressed. A cancelled, failed or abandoned
 * attempt, and a button pressed and then a different method used, record nothing. The pending note is used up
 * either way.
 */
export async function confirmProviderLogin(): Promise<void> {
  const attempt = takeProviderAttempt()
  if (!attempt) return
  try {
    const current = (await api.sessions()).items.find((session) => session.current)
    // A minute of slack for clocks that differ between this browser and the server.
    if (current?.authMethod === attempt.provider && Date.parse(current.createdAt) >= attempt.at - 60_000) setLastMethod(attempt.provider)
  } catch {
    // Not confirmed: nothing is remembered.
  }
}
