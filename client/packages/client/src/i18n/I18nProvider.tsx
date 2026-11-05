import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react'
import { initialLocale, saveLocale } from './locale'
import { messages, type Locale, type MessageKey } from './messages'

type I18nValue = {
  locale: Locale
  setLocale: (locale: Locale) => void
  t: (key: MessageKey, vars?: Record<string, string>) => string
}

const I18nContext = createContext<I18nValue | null>(null)

function format(template: string, vars: Record<string, string> | undefined): string {
  if (!vars) return template
  return template.replace(/\{(\w+)\}/g, (match, key: string) => vars[key] ?? match)
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(initialLocale)

  useEffect(() => {
    document.documentElement.lang = locale
    document.title = messages[locale]['app.name']
  }, [locale])

  const value = useMemo<I18nValue>(() => {
    return {
      locale,
      // Only a choice made here is saved; the detected language is not.
      setLocale: (next: Locale) => {
        setLocaleState(next)
        saveLocale(next)
      },
      t: (key, vars) => format(messages[locale][key], vars),
    }
  }, [locale])

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
