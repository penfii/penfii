/** Text search over key and title, shared by the archived and trash lists. */
export function matchesText(key: string, title: string, query: string): boolean {
  const needle = query.trim().toLowerCase()
  return needle === '' || key.toLowerCase().includes(needle) || title.toLowerCase().includes(needle)
}

/** `PEN-12` after `PEN-9`: keys compare by their number, not as text. */
export function compareKeys(a: string, b: string): number {
  const [pa = '', na = ''] = a.split(/-(?=\d+$)/)
  const [pb = '', nb = ''] = b.split(/-(?=\d+$)/)
  return pa === pb ? Number(na) - Number(nb) : pa.localeCompare(pb)
}
