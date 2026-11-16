import { describe, expect, it } from 'vitest'
import { floatingMenuPosition } from './floatingMenuPosition'

describe('floating menu viewport positioning', () => {
  const viewport = { width: 800, height: 600 }
  it('opens below and keeps right edge inside the viewport', () => {
    expect(floatingMenuPosition({ top: 40, right: 790, bottom: 64, left: 760, width: 30, height: 24 }, { width: 220, height: 140 }, viewport)).toEqual({ top: 68, left: 570, maxHeight: 584, maxWidth: 784 })
  })
  it('opens above when the trigger is near the bottom', () => {
    expect(floatingMenuPosition({ top: 550, right: 250, bottom: 578, left: 220, width: 30, height: 28 }, { width: 220, height: 160 }, viewport)).toEqual({ top: 386, left: 30, maxHeight: 584, maxWidth: 784 })
  })
  it('limits a menu taller than the viewport and clamps its origin', () => {
    expect(floatingMenuPosition({ top: 1, right: 10, bottom: 20, left: 0, width: 10, height: 19 }, { width: 900, height: 900 }, viewport)).toEqual({ top: 8, left: 8, maxHeight: 584, maxWidth: 784 })
  })
  it('fits inside a narrow and short viewport', () => {
    expect(floatingMenuPosition({ top: 30, right: 18, bottom: 40, left: 8, width: 10, height: 10 }, { width: 220, height: 140 }, { width: 100, height: 80 })).toEqual({ top: 8, left: 8, maxHeight: 64, maxWidth: 84 })
  })
})
