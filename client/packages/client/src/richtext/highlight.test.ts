import { describe, expect, it } from 'vitest'
import { highlightCode, resolveHighlightLanguage } from './highlight'

const SAMPLES: Record<string, string> = {
  javascript: 'const n = 1;',
  typescript: 'const userId = "123";\nconsole.log(userId);',
  jsx: 'const View = () => <div />;',
  tsx: 'const View = (): string => <div />;',
  html: '<p class="name">hello</p>',
  css: 'body { color: red; }',
  json: '{ "ok": true }',
  bash: 'echo "$HOME"',
  python: 'def hi():\n    return 1',
  rust: 'fn main() { let x = 1; }',
  sql: 'select id from users;',
  yaml: 'name: penfii\ncount: 1',
  markdown: '# Hello',
}

describe('highlightCode', () => {
  it('colors the supported languages and keeps the source text', async () => {
    for (const [language, source] of Object.entries(SAMPLES)) {
      const lines = await highlightCode(source, language, 'light')
      const text = lines.map((line) => line.map((token) => token.text).join('')).join('\n')
      expect(text, language).toBe(source)
      expect(lines.some((line) => line.some((token) => token.color)), language).toBe(true)
    }
  })

  it('accepts the common aliases', async () => {
    const lines = await highlightCode('echo hi', 'shell', 'light')
    expect(lines.some((line) => line.some((token) => token.color))).toBe(true)
  })

  it('renders an unknown language as the original plaintext', async () => {
    const source = 'const kept = true;'
    const lines = await highlightCode(source, 'unknown-language', 'light')
    expect(lines.flat().map((token) => token.text).join('')).toBe(source)
    expect(lines.flat().every((token) => token.color === undefined)).toBe(true)
  })

  it('treats plaintext and text as plain', () => {
    expect(resolveHighlightLanguage('plaintext', ['typescript'])).toBe('plaintext')
    expect(resolveHighlightLanguage('text', ['typescript'])).toBe('plaintext')
    expect(resolveHighlightLanguage(null, ['typescript'])).toBe('plaintext')
    expect(resolveHighlightLanguage('unknown-language', ['typescript', 'bash'])).toBe('plaintext')
  })

  it('uses a different palette for the dark theme', async () => {
    const source = 'const userId = "123";'
    const light = await highlightCode(source, 'typescript', 'light')
    const dark = await highlightCode(source, 'typescript', 'dark')
    const lightColor = light.flat().find((token) => token.text === 'const')?.color
    const darkColor = dark.flat().find((token) => token.text === 'const')?.color
    expect(lightColor).toBe('#D73A49')
    expect(darkColor).toBe('#F97583')
  })
})
