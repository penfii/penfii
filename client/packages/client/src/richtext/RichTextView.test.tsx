import { render, waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { RichTextDoc } from '@penfii/contracts'
import { I18nProvider } from '../i18n/I18nProvider'
import { LOCALE_STORAGE_KEY } from '../i18n/messages'
import { RichTextView } from './RichTextView'

function renderView(doc: RichTextDoc) {
  localStorage.setItem(LOCALE_STORAGE_KEY, 'ko-KR')
  return render(
    <I18nProvider>
      <RichTextView doc={doc} />
    </I18nProvider>,
  )
}

describe('RichTextView', () => {
  it('does not emit raw HTML from a text node that contains a script tag', () => {
    const doc: RichTextDoc = {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [{ type: 'text', text: '<script>alert(1)</script>' }],
        },
      ],
    }
    const { container } = renderView(doc)
    expect(container.querySelector('script')).toBeNull()
    expect(container.innerHTML.includes('<script')).toBe(false)
    expect(container.textContent).toContain('<script>alert(1)</script>')
  })

  it('highlights a typescript code block as text, not as html', async () => {
    const doc: RichTextDoc = {
      type: 'doc',
      content: [
        {
          type: 'codeBlock',
          attrs: { language: 'typescript' },
          content: [{ type: 'text', text: 'const userId = "123";\nconsole.log(userId);' }],
        },
      ],
    }
    const { container } = renderView(doc)
    await waitFor(() => {
      expect(container.querySelector('code span[style]')?.getAttribute('style')).toContain('color')
    })
    expect(container.querySelector('.code-language')?.textContent).toBe('TypeScript')
    expect(container.textContent).toContain('const userId = "123";')
    expect(container.querySelector('script')).toBeNull()
  })

  it('shows an unknown language as source and keeps html source inert', async () => {
    const source = '<script>alert("test")</script>'
    const doc: RichTextDoc = {
      type: 'doc',
      content: [
        {
          type: 'codeBlock',
          attrs: { language: 'unknown-language' },
          content: [{ type: 'text', text: source }],
        },
        {
          type: 'codeBlock',
          attrs: { language: 'html' },
          content: [{ type: 'text', text: source }],
        },
      ],
    }
    const { container } = renderView(doc)
    await waitFor(() => {
      expect(container.querySelectorAll('code span[style]').length).toBeGreaterThan(0)
    })
    const blocks = [...container.querySelectorAll('.code-block')]
    expect(blocks[0]?.querySelector('.code-language')?.textContent).toBe('unknown-language')
    expect(blocks[0]?.querySelector('span[style]')).toBeNull()
    expect(blocks[0]?.textContent).toContain(source)
    expect(blocks[1]?.querySelector('.code-language')?.textContent).toBe('HTML')
    expect(blocks[1]?.textContent).toContain(source)
    expect(container.querySelector('script')).toBeNull()
    expect(container.innerHTML.includes('<script')).toBe(false)
  })
})
