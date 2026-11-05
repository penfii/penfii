import { isLocale, LOCALE_STORAGE_KEY, type Locale } from './messages'

/** What a visitor gets when they chose nothing and their browser asks for nothing Penfii has. */
export const FALLBACK_LOCALE: Locale = 'en-US'

/**
 * The first language the browser lists (they are in order of preference) that Penfii is translated
 * into. `ko`, `ko-KR` and `ko-Hang` are Korean; `en` and any `en-*` are English; the rest is skipped.
 */
export function detectLocale(preferred: readonly string[]): Locale {
  for (const tag of preferred) {
    const language = tag.trim().toLowerCase().split(/[-_]/)[0]
    if (language === 'ko') return 'ko-KR'
    if (language === 'en') return 'en-US'
  }
  return FALLBACK_LOCALE
}

function browserLanguages(): readonly string[] {
  if (typeof navigator === 'undefined') return []
  if (navigator.languages && navigator.languages.length > 0) return navigator.languages
  return navigator.language ? [navigator.language] : []
}

/** The language the person picked and saved, or `null` when they never did. */
export function savedLocale(): Locale | null {
  try {
    const stored = localStorage.getItem(LOCALE_STORAGE_KEY)
    return isLocale(stored) ? stored : null
  } catch {
    return null
  }
}

/** A saved choice wins; without one, the browser's language; without that, English. */
export function initialLocale(): Locale {
  return savedLocale() ?? detectLocale(browserLanguages())
}

/** Saves a choice the person made. A language that was only detected is never saved. */
export function saveLocale(locale: Locale): void {
  try {
    localStorage.setItem(LOCALE_STORAGE_KEY, locale)
  } catch {
    // Storage may be blocked; the choice still applies until the page is closed.
  }
}
