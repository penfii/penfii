import { isApiError, isDbUnavailable, isNotFound } from '@penfii/api-client'

export function errorText(error: unknown, fallback: string, unavailable = fallback): string {
  if (isNotFound(error)) return fallback
  if (isDbUnavailable(error)) return unavailable
  if (isApiError(error) && error.message.trim().length > 0) return error.message
  return fallback
}
