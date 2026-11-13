import { describe, expect, it } from 'vitest'
import { isImeKey } from './ime'

describe('isImeKey', () => {
  it('is true while the browser says a composition is running', () => {
    expect(isImeKey({ isComposing: true })).toBe(true)
    expect(isImeKey({ nativeEvent: { isComposing: true } })).toBe(true)
  })

  it('is true for the Safari Enter that arrives after the composition ended (keyCode 229)', () => {
    expect(isImeKey({ isComposing: false, keyCode: 229 })).toBe(true)
    expect(isImeKey({ nativeEvent: { isComposing: false, keyCode: 229 } })).toBe(true)
  })

  it('is false for an ordinary key', () => {
    expect(isImeKey({ isComposing: false, keyCode: 13 })).toBe(false)
    expect(isImeKey({ nativeEvent: { isComposing: false, keyCode: 13 } })).toBe(false)
    expect(isImeKey({})).toBe(false)
  })
})
