const MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent)

/** Shortcuts registered by the editor. Labels are only produced for these chords. */
export const TOOLBAR_SHORTCUTS = {
  bold: 'Mod-b',
  italic: 'Mod-i',
  strike: 'Mod-Shift-s',
  blockquote: 'Mod-Shift-b',
  bulletList: 'Mod-Shift-8',
  orderedList: 'Mod-Shift-7',
  link: 'Mod-k',
  code: 'Mod-e',
  codeBlock: 'Mod-Alt-c',
} as const

export function shortcutLabel(chord: string, mac = MAC): string {
  const parts = chord.split('-')
  const key = parts.at(-1) ?? ''
  const mods: string[] = []
  if (parts.includes('Mod')) mods.push(mac ? 'Cmd' : 'Ctrl')
  if (parts.includes('Shift')) mods.push(mac ? 'Shift' : 'Shift')
  if (parts.includes('Alt')) mods.push(mac ? 'Option' : 'Alt')
  const shown = key.length === 1 ? key.toUpperCase() : key
  return [...mods, shown].join('+')
}

export function commandTitle(name: string, chord: string, mac = MAC): string {
  return `${name} · ${shortcutLabel(chord, mac)}`
}

export function ariaShortcut(chord: string): string {
  return chord.replaceAll('Mod', 'Control').replaceAll('-', '+')
}
