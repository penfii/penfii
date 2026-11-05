import { isApiError, isNotFound } from '@penfii/api-client'

export function errorText(error: unknown, fallback: string): string {
  if (isNotFound(error)) return fallback
  if (isApiError(error) && error.message.trim().length > 0) return error.message
  return fallback
}
