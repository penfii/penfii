/// <reference types="vite/client" />
import { describe, expect, it, vi } from 'vitest'

/*
 * Every catalog is spread into one object, so a key written in two places silently loses one of
 * them (the Timeline once showed the chat's "No messages" because both used `timeline.empty`).
 * Each key must appear exactly twice across all catalog files: once in Korean and once in English.
 */
const sources = import.meta.glob('../**/*essages*.ts', { query: '?raw', import: 'default', eager: true }) as Record<string, string>

describe('message catalogs', () => {
  it('write every key once per language', () => {
    const counts = new Map<string, number>()
    for (const text of Object.values(sources)) {
      for (const match of text.matchAll(/^\s*'([A-Za-z0-9_.]+)'\s*:/gm)) counts.set(match[1], (counts.get(match[1]) ?? 0) + 1)
    }
    const doubled = [...counts].filter(([, count]) => count > 2).map(([key]) => key)
    expect(Object.keys(sources).length).toBeGreaterThan(5)
    expect(doubled).toEqual([])
  })

  it('give both languages exactly the same keys', async () => {
    const { loadCatalog } = await import('./catalog')
    const [ko, en] = await Promise.all([loadCatalog('ko-KR'), loadCatalog('en-US')])
    expect(Object.keys(en).sort()).toEqual(Object.keys(ko).sort())
    expect(Object.keys(ko).length).toBeGreaterThan(1000)
  })
})

describe('catalog loading', () => {
  it('fetches a language once, only when asked, and keeps it for the synchronous reads', async () => {
    vi.resetModules()
    const fresh = await import('./catalog')
    expect(fresh.loadedCatalog('en-US')).toBeUndefined()
    const first = fresh.loadCatalog('en-US')
    expect(fresh.loadCatalog('en-US')).toBe(first)
    const catalog = await first
    expect(fresh.loadedCatalog('en-US')).toBe(catalog)
    // Nothing asked for Korean, so it was not fetched into this module instance.
    expect(fresh.loadedCatalog('ko-KR')).toBeUndefined()
  })
})
