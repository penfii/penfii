const FENCE = /^```([A-Za-z0-9_+#-]*)[ \t]*$/

/** A paragraph that is only an opening fence, such as ``` or ```rust. */
export function fenceOpening(text: string): { language: string | null } | null {
  const match = FENCE.exec(text)
  if (!match) return null
  const language = match[1] ?? ''
  return { language: language.length > 0 ? language : null }
}

export function containsFence(text: string): boolean {
  return /(^|\n)```/.test(text)
}
