import type { TableColumn, ViewLayout } from '@penfii/contracts'
import { TABLE_COLUMNS } from './viewModel'

/**
 * How one person looks at one View: the layout, the Table's density and its columns. It is theirs alone and stays on
 * this device (the product keeps no per-person setting on the server for it), so changing it never touches the shared
 * View and never shows as an unsaved change. What it does not say falls back to the View's own saved display and then
 * to the product's default.
 */
export type DisplayPrefs = {
  layout?: ViewLayout
  density?: 'basic' | 'compact'
  columns?: TableColumn[]
}

const PREFS_KEY = 'penfii.work.display.'

/** One entry per person, place and View: two people on one browser, or the same View opened by two people, never share it. */
export function prefsKey(userId: string, targetKey: string, viewId: string | null): string {
  return `${PREFS_KEY}${userId}.${targetKey}.${viewId ?? 'all'}`
}

export function readPrefs(key: string): DisplayPrefs {
  try {
    const raw = localStorage.getItem(key)
    if (!raw) return {}
    const value = JSON.parse(raw) as Record<string, unknown>
    const prefs: DisplayPrefs = {}
    if (value.layout === 'BOARD' || value.layout === 'TABLE') prefs.layout = value.layout
    if (value.density === 'basic' || value.density === 'compact') prefs.density = value.density
    if (Array.isArray(value.columns)) prefs.columns = value.columns.filter((column): column is TableColumn => TABLE_COLUMNS.some((known) => known === column))
    return prefs
  } catch {
    return {}
  }
}

export function writePrefs(key: string, prefs: DisplayPrefs): void {
  try {
    if (prefs.layout === undefined && prefs.density === undefined && prefs.columns === undefined) localStorage.removeItem(key)
    else localStorage.setItem(key, JSON.stringify(prefs))
  } catch {
    // The choice still applies for this visit.
  }
}

export function hasPrefs(prefs: DisplayPrefs): boolean {
  return prefs.layout !== undefined || prefs.density !== undefined || prefs.columns !== undefined
}
