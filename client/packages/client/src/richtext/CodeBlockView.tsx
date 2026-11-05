import { useEffect, useRef, useState } from 'react'
import { useI18n } from '../i18n/I18nProvider'
import { codeLanguageLabel } from './codeLabel'
import { highlightCode, type HighlightTheme, type HighlightToken } from './highlight'

export const CODE_COPY_RESET_MS = 1600

type CopyState = 'idle' | 'copied' | 'failed'

function useHighlightTheme(): HighlightTheme {
  const [theme, setTheme] = useState<HighlightTheme>(() =>
    document.documentElement.dataset.theme === 'dark' ? 'dark' : 'light',
  )
  useEffect(() => {
    const root = document.documentElement
    const sync = () => setTheme(root.dataset.theme === 'dark' ? 'dark' : 'light')
    const observer = new MutationObserver(sync)
    observer.observe(root, { attributes: true, attributeFilter: ['data-theme'] })
    return () => observer.disconnect()
  }, [])
  return theme
}

async function writeCodeSource(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch {
    return false
  }
}

export function CodeBlockView({ language, text }: { language: string | null; text: string }) {
  const { t } = useI18n()
  const theme = useHighlightTheme()
  const [lines, setLines] = useState<HighlightToken[][] | null>(null)
  const [copyState, setCopyState] = useState<CopyState>('idle')
  const resetTimer = useRef<number | null>(null)
  const label = codeLanguageLabel(language)

  useEffect(() => {
    let current = true
    setLines(null)
    void highlightCode(text, language, theme).then((next) => {
      if (current) setLines(next)
    })
    return () => {
      current = false
    }
  }, [text, language, theme])

  useEffect(() => () => {
    if (resetTimer.current != null) window.clearTimeout(resetTimer.current)
  }, [])

  function scheduleReset() {
    if (resetTimer.current != null) window.clearTimeout(resetTimer.current)
    resetTimer.current = window.setTimeout(() => {
      resetTimer.current = null
      setCopyState('idle')
    }, CODE_COPY_RESET_MS)
  }

  async function copy() {
    const copied = await writeCodeSource(text)
    setCopyState(copied ? 'copied' : 'failed')
    scheduleReset()
  }

  const copyLabel = copyState === 'copied'
    ? t('code.copied')
    : copyState === 'failed'
      ? t('code.copyFailed')
      : t('code.copy')

  return (
    <div className="code-block">
      <div className="code-block-header">
        {label ? <span className="code-language">{label}</span> : null}
        <button
          type="button"
          className="code-copy"
          data-state={copyState}
          aria-live="polite"
          onClick={() => void copy()}
        >
          {copyLabel}
        </button>
      </div>
      <div className="code-block-content">
        <pre>
          <code>
            {lines
              ? lines.map((line, index) => (
                  <span key={index}>
                    {index > 0 ? '\n' : null}
                    {line.map((token, tokenIndex) =>
                      token.color ? (
                        <span key={tokenIndex} style={{ color: token.color }}>
                          {token.text}
                        </span>
                      ) : (
                        token.text
                      ),
                    )}
                  </span>
                ))
              : text}
          </code>
        </pre>
      </div>
    </div>
  )
}
