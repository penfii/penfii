/// <reference types="vite/client" />
import { describe, expect, it } from 'vitest'

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
})
