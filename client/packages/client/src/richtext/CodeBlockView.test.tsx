import { act, fireEvent, render, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { RichTextDoc } from '@penfii/contracts'
import { I18nProvider } from '../i18n/I18nProvider'
import { LOCALE_STORAGE_KEY } from '../i18n/messages'
import { CODE_COPY_RESET_MS, CodeBlockView } from './CodeBlockView'
import { RichTextView } from './RichTextView'

function renderBlock(language: string | null, text: string) {
  localStorage.setItem(LOCALE_STORAGE_KEY, 'ko-KR')
  return render(
    <I18nProvider>
      <CodeBlockView language={language} text={text} />
    </I18nProvider>,
  )
}

function renderDoc(doc: RichTextDoc) {
  localStorage.setItem(LOCALE_STORAGE_KEY, 'ko-KR')
  return render(
    <I18nProvider>
      <RichTextView doc={doc} />
    </I18nProvider>,
  )
}

describe('CodeBlockView', () => {
  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('copies the original source, including line breaks, then returns to Copy', async () => {
    const source = "const userId = '123';\nalpha\nbeta\ngamma"
    const writeText = vi.fn().mockResolvedValue(undefined)
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    vi.useFakeTimers()
    const { container } = renderBlock('typescript', source)
    const button = container.querySelector('button')
    expect(button?.tagName).toBe('BUTTON')
    expect(button?.getAttribute('type')).toBe('button')
    expect(button?.textContent).toBe('복사')
    await act(async () => {
      fireEvent.click(button as HTMLButtonElement)
      await Promise.resolve()
    })
    expect(writeText).toHaveBeenCalledWith(source)
    expect(button?.textContent).toBe('복사됨')
    await act(async () => {
      vi.advanceTimersByTime(CODE_COPY_RESET_MS)
    })
    expect(button?.textContent).toBe('복사')
  })

  it('shows a failed state and keeps the source when the clipboard rejects the write', async () => {
    const source = 'const kept = true;'
    const writeText = vi.fn().mockRejectedValue(new Error('denied'))
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    const { container } = renderBlock(null, source)
    const button = container.querySelector('button') as HTMLButtonElement
    fireEvent.click(button)
    await waitFor(() => expect(button.textContent).toBe('복사 실패'))
    expect(container.querySelector('.code-language')).toBeNull()
    expect(container.querySelector('code')?.textContent).toBe(source)
    expect(container.querySelector('script')).toBeNull()
  })

  it('labels known aliases and leaves a missing language blank', async () => {
    const labeled = renderBlock('js', 'const n = 1;')
    expect(labeled.container.querySelector('.code-language')?.textContent).toBe('JavaScript')
    await waitFor(() => {
      expect(labeled.container.querySelector('code span[style]')).not.toBeNull()
    })
    labeled.unmount()
    const blank = renderBlock(null, 'alpha\nbeta')
    expect(blank.container.querySelector('.code-language')).toBeNull()
    expect(blank.container.querySelector('button')?.textContent).toBe('복사')
  })

  it('keeps an unknown language as plaintext and copies the html source as text', async () => {
    const source = '<script>alert("test")</script>'
    const writeText = vi.fn().mockResolvedValue(undefined)
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    const doc: RichTextDoc = {
      type: 'doc',
      content: [
        { type: 'codeBlock', attrs: { language: 'unknown-language' }, content: [{ type: 'text', text: source }] },
        { type: 'codeBlock', attrs: { language: 'html' }, content: [{ type: 'text', text: source }] },
      ],
    }
    const { container } = renderDoc(doc)
    await waitFor(() => {
      expect(container.querySelectorAll('code span[style]').length).toBeGreaterThan(0)
    })
    const blocks = [...container.querySelectorAll('.code-block')]
    expect(blocks[0]?.querySelector('.code-language')?.textContent).toBe('unknown-language')
    expect(blocks[0]?.querySelector('span[style]')).toBeNull()
    expect(blocks[1]?.querySelector('.code-language')?.textContent).toBe('HTML')
    const htmlButton = blocks[1]?.querySelector('button') as HTMLButtonElement
    fireEvent.click(htmlButton)
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(source))
    expect(container.querySelector('script')).toBeNull()
    expect(container.innerHTML.includes('<script')).toBe(false)
  })

  it('caps the code area and preserves long lines for scrolling', () => {
    const long = Array.from({ length: 80 }, (_, index) => `const line${String(index)} = ${String(index)};`).join('\n')
    const wide = `const veryLongVariableName = "${'x'.repeat(240)}";`
    const { container } = renderBlock('typescript', `${long}\n${wide}`)
    const header = container.querySelector('.code-block-header')
    const content = container.querySelector('.code-block-content')
    const code = container.querySelector('code')
    expect(header?.nextElementSibling).toBe(content)
    expect(content?.contains(header)).toBe(false)
    expect(content?.className).toBe('code-block-content')
    expect(code?.textContent).toContain(wide)
    expect(code?.textContent?.split('\n')).toHaveLength(81)
  })
})
