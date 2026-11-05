import { describe, expect, it } from 'vitest'
import { containsFence, fenceOpening } from './fence'
import { pastedLink } from './linkPaste'
import { commandTitle, shortcutLabel } from './shortcuts'

describe('fenced code openings', () => {
  it('reads a language and a bare fence', () => {
    expect(fenceOpening('```javascript')).toEqual({ language: 'javascript' })
    expect(fenceOpening('```typescript')).toEqual({ language: 'typescript' })
    expect(fenceOpening('```rust')).toEqual({ language: 'rust' })
    expect(fenceOpening('```')).toEqual({ language: null })
    expect(fenceOpening('```c++')).toEqual({ language: 'c++' })
    expect(fenceOpening('const x = 1')).toBeNull()
    expect(containsFence('```rust\nfn main() {}\n```')).toBe(true)
  })
})

describe('link paste', () => {
  it('accepts one http URL and rejects other text', () => {
    expect(pastedLink(' https://openai.com ')).toBe('https://openai.com')
    expect(pastedLink('OpenAI official website')).toBeNull()
    expect(pastedLink('javascript:alert(1)')).toBeNull()
    expect(pastedLink('https://user:pass@example.com')).toBeNull()
  })
})

describe('shortcut labels', () => {
  it('uses Ctrl on Linux and Cmd on macOS', () => {
    expect(shortcutLabel('Mod-b', false)).toBe('Ctrl+B')
    expect(shortcutLabel('Mod-Shift-s', false)).toBe('Ctrl+Shift+S')
    expect(commandTitle('Bold', 'Mod-b', false)).toBe('Bold · Ctrl+B')
    expect(shortcutLabel('Mod-k', true)).toBe('Cmd+K')
  })
})
