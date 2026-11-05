import { isSafeHttpUrl } from './url'

/** A single http(s) URL. Surrounding space is ignored. Anything else is ordinary text. */
export function pastedLink(text: string): string | null {
  const href = text.trim()
  if (href.length === 0 || /\s/.test(href)) return null
  return isSafeHttpUrl(href) ? href : null
}
