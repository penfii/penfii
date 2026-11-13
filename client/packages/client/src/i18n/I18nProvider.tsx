import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react'
import { loadCatalog, loadedCatalog } from './catalog'
import { initialLocale, saveLocale } from './locale'
import type { Locale, MessageKey } from './messages'

type I18nValue = {
  locale: Locale
  setLocale: (locale: Locale) => void
  t: (key: MessageKey, vars?: Record<string, string>) => string
  /** Whether `key` is a text this build knows (an audit entry may carry an action from a newer server). */
  has: (key: string) => key is MessageKey
}

const I18nContext = createContext<I18nValue | null>(null)

function format(template: string, vars: Record<string, string> | undefined): string {
  if (!vars) return template
  return template.replace(/\{(\w+)\}/g, (match, key: string) => vars[key] ?? match)
}

/**
 * Shows the app in one language. The texts of a language arrive as their own chunk; the entry point
 * waits for the first one (`loadCatalog`) so nothing is drawn without text. Choosing the other language
 * fetches its catalog first and switches when it is there, so the screen never shows keys.
 */
export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(initialLocale)
  const [, setReady] = useState(0)
  const [failed, setFailed] = useState(false)
  const catalog = loadedCatalog(locale)

  useEffect(() => {
    if (catalog) return undefined
    let live = true
    // Rendered once the catalog is in; if the fetch fails nothing can be drawn, and the error surfaces.
    void loadCatalog(locale).then(
      () => { if (live) setReady((count) => count + 1) },
      () => { if (live) setFailed(true) },
    )
    return () => { live = false }
  }, [catalog, locale])

  useEffect(() => {
    if (!catalog) return
    document.documentElement.lang = locale
    document.title = catalog['app.name']
  }, [catalog, locale])

  const value = useMemo<I18nValue | null>(() => {
    if (!catalog) return null
    return {
      locale,
      // Only a choice made here is saved; the detected language is not.
      setLocale: (next: Locale) => {
        const apply = () => {
          setLocaleState(next)
          saveLocale(next)
        }
        // A catalog that is already here switches at once; otherwise the old language stays until the new one arrives.
        if (loadedCatalog(next)) apply()
        else void loadCatalog(next).then(apply, () => undefined)
      },
      t: (key, vars) => format(catalog[key], vars),
      has: (key): key is MessageKey => key in catalog,
    }
  }, [catalog, locale])

  if (!value) {
    // The texts could not be fetched (offline, or a new version replaced the files this page names): nothing can be drawn
    // in a language, so say so in both, with the one thing that helps.
    return failed ? (
      <div role="alert" style={{ padding: 24, fontFamily: 'sans-serif' }}>
        <p>The page could not be loaded. / 화면을 불러오지 못했습니다.</p>
        <button type="button" onClick={() => window.location.reload()}>Reload / 새로고침</button>
      </div>
    ) : null
  }
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>
}

export function useI18n(): I18nValue {
  const value = useContext(I18nContext)
  if (!value) throw new Error('I18nProvider is required')
  return value
}

export function LocaleToggle() {
  const { locale, setLocale, t } = useI18n()
  const next = locale === 'ko-KR' ? 'en-US' : 'ko-KR'
  return (
    <button type="button" className="btn btn-secondary" onClick={() => setLocale(next)}>
      {next === 'en-US' ? t('locale.en') : t('locale.ko')}
    </button>
  )
}
