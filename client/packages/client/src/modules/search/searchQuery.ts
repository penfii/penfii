/**
 * Small helpers over the search text. The filter chips edit the same text a person can type, so a
 * chip and its typed form always mean the same thing.
 */

/** Splits on whitespace but keeps "quoted text" and key:"quoted value" together. */
export function tokens(q: string): string[] {
  const out: string[] = []
  let current = ''
  let quoted = false
  for (const ch of q) {
    if (ch === '"') quoted = !quoted
    if (/\s/.test(ch) && !quoted) {
      if (current) out.push(current)
      current = ''
    } else {
      current += ch
    }
  }
  if (current) out.push(current)
  return out
}

function keyOf(token: string): string | null {
  const match = /^([a-z]+):/i.exec(token)
  return match ? match[1].toLowerCase() : null
}

export function hasToken(q: string, token: string): boolean {
  return tokens(q).some((item) => item.toLowerCase() === token.toLowerCase())
}

/** Adds the token if it is missing, removes it if present. */
export function toggleToken(q: string, token: string): string {
  const list = tokens(q)
  const at = list.findIndex((item) => item.toLowerCase() === token.toLowerCase())
  if (at >= 0) list.splice(at, 1)
  else list.push(token)
  return list.join(' ')
}

/** Replaces every `key:` token with one `key:value`, or removes them when value is null. */
export function setKey(q: string, key: string, value: string | null): string {
  const list = tokens(q).filter((item) => keyOf(item) !== key)
  if (value) list.push(`${key}:${/\s/.test(value) ? `"${value}"` : value}`)
  return list.join(' ')
}

export function valueOf(q: string, key: string): string | null {
  const token = tokens(q).find((item) => keyOf(item) === key)
  if (!token) return null
  return token.slice(key.length + 1).replace(/^"|"$/g, '')
}

/** Replaces one offending token (used to settle an ambiguous name by its id). */
export function replaceToken(q: string, token: string, replacement: string): string {
  return tokens(q).map((item) => (item === token ? replacement : item)).join(' ')
}
