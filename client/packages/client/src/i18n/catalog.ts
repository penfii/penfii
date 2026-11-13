import { initialLocale } from './locale'
import type { Catalog, Locale } from './messages'

/**
 * The texts of each language are separate chunks: a visitor downloads the one they read, and the other
 * only when they switch to it. A catalog that has been loaded is kept here so `t` stays synchronous.
 */
const loaders: Record<Locale, () => Promise<Catalog>> = {
  'ko-KR': () => import('./messages.ko').then((module) => module.ko),
  'en-US': () => import('./messages.en').then((module) => module.en),
}

const loaded: Partial<Record<Locale, Catalog>> = {}
const pending: Partial<Record<Locale, Promise<Catalog>>> = {}

/** The catalog of a language that is already loaded, or `undefined` while it still has to be fetched. */
export function loadedCatalog(locale: Locale): Catalog | undefined {
  return loaded[locale]
}

export function loadCatalog(locale: Locale): Promise<Catalog> {
  const have = loaded[locale]
  if (have) return Promise.resolve(have)
  pending[locale] ??= loaders[locale]().then(
    (catalog) => {
      loaded[locale] = catalog
      return catalog
    },
    (error: unknown) => {
      // A failed fetch must not be remembered: the next try asks the network again.
      delete pending[locale]
      throw error
    },
  )
  return pending[locale]
}

/** Fetches the catalog of the language the app starts in, so the first render already has its texts. */
export function loadInitialCatalog(): Promise<Catalog> {
  return loadCatalog(initialLocale())
}
