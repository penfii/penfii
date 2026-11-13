import type { ko } from './messages.ko'

export const LOCALE_STORAGE_KEY = 'penfii.locale'

export type Locale = 'ko-KR' | 'en-US'

/** Every text key. The Korean catalog defines the set; the English one must have exactly the same keys. */
export type MessageKey = keyof typeof ko

export type Catalog = Record<MessageKey, string>

export function isLocale(value: string | null): value is Locale {
  return value === 'ko-KR' || value === 'en-US'
}
