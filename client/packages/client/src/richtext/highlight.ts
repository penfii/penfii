import { createHighlighterCore, isPlainLang, type HighlighterCore } from 'shiki/core'

export type HighlightTheme = 'light' | 'dark'

export interface HighlightToken {
  text: string
  color?: string
}

const ALIASES: Record<string, string> = {
  text: 'plaintext',
  txt: 'plaintext',
  plain: 'plaintext',
  js: 'javascript',
  ts: 'typescript',
  sh: 'bash',
  shell: 'bash',
  zsh: 'bash',
  py: 'python',
  yml: 'yaml',
  md: 'markdown',
}

const THEME_NAMES = {
  light: 'github-light',
  dark: 'github-dark',
} as const

const SAFE_COLOR = /^#[0-9A-Fa-f]{6}$/
const CACHE_LIMIT = 80

let highlighterPromise: Promise<HighlighterCore> | null = null
const cache = new Map<string, HighlightToken[][]>()

function highlighter(): Promise<HighlighterCore> {
  // The grammars are most of the editor's weight; they load with the first code block, not with the app.
  highlighterPromise ??= createHighlighterCore({
    themes: [import('@shikijs/themes/github-light'), import('@shikijs/themes/github-dark')],
    langs: [
      import('@shikijs/langs/javascript'),
      import('@shikijs/langs/typescript'),
      import('@shikijs/langs/jsx'),
      import('@shikijs/langs/tsx'),
      import('@shikijs/langs/html'),
      import('@shikijs/langs/css'),
      import('@shikijs/langs/json'),
      import('@shikijs/langs/bash'),
      import('@shikijs/langs/python'),
      import('@shikijs/langs/rust'),
      import('@shikijs/langs/sql'),
      import('@shikijs/langs/yaml'),
      import('@shikijs/langs/markdown'),
    ],
    engine: import('shiki/engine/javascript').then(({ createJavaScriptRegexEngine }) => createJavaScriptRegexEngine()),
  })
  return highlighterPromise
}

export function resolveHighlightLanguage(language: string | null, loaded: readonly string[]): string {
  const raw = language?.trim().toLowerCase() ?? ''
  if (raw.length === 0 || isPlainLang(raw)) return 'plaintext'
  const candidate = ALIASES[raw] ?? raw
  if (isPlainLang(candidate) || loaded.includes(candidate)) return candidate
  return 'plaintext'
}

function remember(key: string, lines: HighlightToken[][]): HighlightToken[][] {
  if (cache.has(key)) cache.delete(key)
  cache.set(key, lines)
  while (cache.size > CACHE_LIMIT) {
    const oldest = cache.keys().next().value
    if (oldest === undefined) break
    cache.delete(oldest)
  }
  return lines
}

function plainLines(source: string): HighlightToken[][] {
  return source.split('\n').map((line) => [{ text: line }])
}

export async function highlightCode(
  source: string,
  language: string | null,
  theme: HighlightTheme,
): Promise<HighlightToken[][]> {
  try {
    const engine = await highlighter()
    const lang = resolveHighlightLanguage(language, engine.getLoadedLanguages())
    const themeName = THEME_NAMES[theme]
    const key = `${themeName}\0${lang}\0${source}`
    const hit = cache.get(key)
    if (hit) {
      cache.delete(key)
      cache.set(key, hit)
      return hit
    }
    const { tokens } = engine.codeToTokens(source, { lang, theme: themeName })
    const lines = tokens.map((line) =>
      line.map((token) => {
        const color = token.color && SAFE_COLOR.test(token.color) ? token.color : undefined
        return color ? { text: token.content, color } : { text: token.content }
      }),
    )
    return remember(key, lines)
  } catch {
    return plainLines(source)
  }
}
