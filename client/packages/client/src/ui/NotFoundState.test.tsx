import { render } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { describe, expect, it } from 'vitest'
import { I18nProvider } from '../i18n/I18nProvider'
import { LOCALE_STORAGE_KEY } from '../i18n/messages'
import { NotFoundState } from './NotFoundState'

function view(escape: boolean) {
  localStorage.setItem(LOCALE_STORAGE_KEY, 'en-US')
  return render(
    <MemoryRouter>
      <I18nProvider>
        <NotFoundState escape={escape} />
      </I18nProvider>
    </MemoryRouter>,
  )
}

describe('the not-found state', () => {
  it('offers a way out when nothing else on the page can navigate', () => {
    const { container } = view(true)
    const link = container.querySelector('a')
    expect(link?.getAttribute('href')).toBe('/')
    expect(link?.textContent).toBe('Go to your workspaces')
  })

  it('stays a plain message inside a pane that has its own navigation', () => {
    const { container } = view(false)
    expect(container.querySelector('a')).toBeNull()
    expect(container.textContent).toBe('Not found')
  })
})
