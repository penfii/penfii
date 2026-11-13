import { fireEvent, render, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { RichTextDoc } from '@penfii/contracts'
import { I18nProvider } from '../i18n/I18nProvider'
import { LOCALE_STORAGE_KEY } from '../i18n/messages'
import { RichTextView } from './RichTextView'
import { RichTextEnvContext, type RichTextEnv } from './env'

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

describe('internal links', () => {
  const here = window.location.origin
  const link = (href: string, text = href): RichTextDoc => ({
    type: 'doc',
    content: [{ type: 'paragraph', content: [{ type: 'text', text, marks: [{ type: 'link', attrs: { href } }] }] }],
  })
  function renderWithEnv(doc: RichTextDoc, env: Partial<RichTextEnv> = {}) {
    localStorage.setItem(LOCALE_STORAGE_KEY, 'en-US')
    const navigate = vi.fn()
    const full: RichTextEnv = {
      workspaceId: 'w1',
      resolver: {
        subscribe: () => () => undefined,
        getVersion: () => 0,
        want: (kind, id) =>
          kind === 'TICKET' && id === 't1'
            ? { kind, id, available: true, title: 'PEN-1 Fix login' }
            : kind === 'DOC' && id === 'gone'
              ? { kind, id, available: false }
              : kind === 'USER' && id === 'u1'
                ? { kind, id, available: true, title: 'Mina' }
                : kind === 'USER' || id === 'hidden'
                  ? { kind, id, available: false }
                  : undefined,
      },
      space: {
        teams: [],
        projects: [{ id: 'p1', name: 'Payments' }],
        channels: [{ id: 'ch1', conversationId: 'c1', name: 'general' }],
        dms: [],
      } as unknown as RichTextEnv['space'],
      navigate,
      ...env,
    }
    const view = render(
      <I18nProvider><RichTextEnvContext.Provider value={full}><RichTextView doc={doc} /></RichTextEnvContext.Provider></I18nProvider>,
    )
    return { ...view, navigate }
  }

  it('shows the target name for a bare in-app URL and keeps the saved URL as the href', () => {
    const url = `${here}/w/w1/projects/p1/work?panel=ticket:t1`
    const { container, navigate } = renderWithEnv(link(url))
    const chip = container.querySelector('a.link-chip') as HTMLAnchorElement
    expect(chip.textContent).toContain('PEN-1 Fix login')
    expect(chip.getAttribute('href')).toBe(url)
    expect(chip.getAttribute('target')).toBeNull()
    fireEvent.click(chip)
    expect(navigate).toHaveBeenCalledWith('/w/w1/projects/p1/work?panel=ticket:t1')
  })

  it('names channels, threads and projects from the person’s own list', () => {
    const channelChip = renderWithEnv(link(`${here}/w/w1/c/c1`)).container.textContent ?? ''
    expect(channelChip).toMatch(/#\s*general/)
    expect(channelChip).not.toMatch(/#\s*#/)
    expect(renderWithEnv(link(`${here}/w/w1/c/c1/t/m1`)).container.textContent).toContain('Thread in # general')
    expect(renderWithEnv(link(`${here}/w/w1/projects/p1`)).container.textContent).toContain('Payments')
  })

  it('does not name what is missing or inaccessible, and does not lose the URL', () => {
    const url = `${here}/w/w1/docs?doc=gone`
    const { container } = renderWithEnv(link(url))
    const chip = container.querySelector('a.link-chip') as HTMLAnchorElement
    expect(chip.textContent).toContain('Link not available')
    expect(chip.getAttribute('href')).toBe(url)
    expect(chip.getAttribute('title')).toBe(url)
    const other = renderWithEnv(link(`${here}/w/w1/c/hidden`)).container.querySelector('a.link-chip') as HTMLAnchorElement
    expect(other.textContent).toContain('Link not available')
  })

  it('does not resolve another workspace’s link, even on the same origin', () => {
    const { container } = renderWithEnv(link(`${here}/w/w2/projects/p1/work?panel=ticket:t1`))
    expect(container.textContent).not.toContain('PEN-1')
    expect(container.textContent).toContain('Link not available')
  })

  it('keeps the words a person wrote and leaves external links alone', () => {
    const own = renderWithEnv(link(`${here}/w/w1/c/c1`, 'our channel'))
    const a = own.container.querySelector('a') as HTMLAnchorElement
    expect(a.textContent).toBe('our channel')
    expect(a.className).not.toContain('link-chip')
    const external = renderWithEnv(link('https://example.com/page')).container.querySelector('a') as HTMLAnchorElement
    expect(external.getAttribute('target')).toBe('_blank')
    expect(external.className).not.toContain('link-chip')
    const otherOrigin = renderWithEnv(link('https://other.test/w/w1/c/c1')).container.querySelector('a') as HTMLAnchorElement
    expect(otherOrigin.getAttribute('target')).toBe('_blank')
  })

  it('leaves new-tab clicks to the browser', () => {
    const { container, navigate } = renderWithEnv(link(`${here}/w/w1/c/c1`))
    const chip = container.querySelector('a.link-chip') as HTMLAnchorElement
    fireEvent.click(chip, { ctrlKey: true })
    fireEvent.click(chip, { metaKey: true })
    fireEvent.click(chip, { button: 1 })
    expect(navigate).not.toHaveBeenCalled()
  })

  it('shows a mentioned person’s name and never the id', () => {
    const doc: RichTextDoc = { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'mention', attrs: { occurrenceId: 'o1', kind: 'USER', userId: 'u1' } }, { type: 'mention', attrs: { occurrenceId: 'o2', kind: 'USER', userId: 'u-gone' } }] }] }
    const { container } = renderWithEnv(doc)
    expect(container.textContent).toContain('@Mina')
    expect(container.textContent).toContain('@Unknown user')
    expect(container.textContent).not.toContain('u-gone')
  })
})
