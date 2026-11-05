export const THEME_KEY = 'penfii.theme'
export type ThemeMode = 'light' | 'dark' | 'system'

export function readTheme(): ThemeMode {
  try {
    const stored = localStorage.getItem(THEME_KEY)
    if (stored === 'light' || stored === 'dark' || stored === 'system') return stored
  } catch {
    // A private window still follows the system setting for this visit.
  }
  return 'system'
}

/** Dark tokens live on `[data-theme="dark"]`. Light is the default `:root`. */
export function applyTheme(mode: ThemeMode): void {
  const systemDark = window.matchMedia('(prefers-color-scheme: dark)').matches
  const dark = mode === 'dark' || (mode === 'system' && systemDark)
  if (dark) document.documentElement.dataset.theme = 'dark'
  else delete document.documentElement.dataset.theme
}

export function saveTheme(mode: ThemeMode): void {
  try {
    localStorage.setItem(THEME_KEY, mode)
  } catch {
    // The choice still applies for this visit.
  }
  applyTheme(mode)
}
