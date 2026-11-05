import { describe, expect, it } from 'vitest'
import { codeLanguageLabel } from './codeLabel'

describe('codeLanguageLabel', () => {
  it('maps stored languages and aliases to display names', () => {
    expect(codeLanguageLabel('typescript')).toBe('TypeScript')
    expect(codeLanguageLabel('ts')).toBe('TypeScript')
    expect(codeLanguageLabel('javascript')).toBe('JavaScript')
    expect(codeLanguageLabel('js')).toBe('JavaScript')
    expect(codeLanguageLabel('python')).toBe('Python')
    expect(codeLanguageLabel('py')).toBe('Python')
    expect(codeLanguageLabel('rust')).toBe('Rust')
    expect(codeLanguageLabel('yml')).toBe('YAML')
    expect(codeLanguageLabel('md')).toBe('Markdown')
    expect(codeLanguageLabel('sh')).toBe('Shell')
    expect(codeLanguageLabel('HTML')).toBe('HTML')
  })

  it('keeps an unknown language visible without inventing a name', () => {
    expect(codeLanguageLabel('unknown-language')).toBe('unknown-language')
  })

  it('returns no label when the block has no language', () => {
    expect(codeLanguageLabel(null)).toBeNull()
    expect(codeLanguageLabel(undefined)).toBeNull()
    expect(codeLanguageLabel('   ')).toBeNull()
  })
})
