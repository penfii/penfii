import { act, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider, LocaleToggle, useI18n } from './I18nProvider'
import { detectLocale, initialLocale } from './locale'
import { LOCALE_STORAGE_KEY } from './messages'

function setBrowser(languages: string[]) {
  Object.defineProperty(navigator, 'languages', { value: languages, configurable: true })
  Object.defineProperty(navigator, 'language', { value: languages[0] ?? '', configurable: true })
}

afterEach(() => {
  vi.restoreAllMocks()
  setBrowser(['ko-KR', 'ko'])
})

describe('detectLocale', () => {
  it('takes the first listed language Penfii is translated into', () => {
    expect(detectLocale(['ko-KR'])).toBe('ko-KR')
    expect(detectLocale(['ko'])).toBe('ko-KR')
    expect(detectLocale(['en-GB'])).toBe('en-US')
    expect(detectLocale(['EN'])).toBe('en-US')
    expect(detectLocale(['fr-FR', 'ko-KR', 'en-US'])).toBe('ko-KR')
    expect(detectLocale(['ja-JP', 'en'])).toBe('en-US')
  })

  it('falls back to English for a language it does not have, or none', () => {
    expect(detectLocale(['ja-JP'])).toBe('en-US')
    expect(detectLocale(['zh-Hans-CN', 'de'])).toBe('en-US')
    expect(detectLocale([])).toBe('en-US')
    expect(detectLocale([''])).toBe('en-US')
  })
})

describe('initialLocale', () => {
  it('prefers what the person saved over the browser', () => {
    setBrowser(['ko-KR'])
    localStorage.setItem(LOCALE_STORAGE_KEY, 'en-US')
    expect(initialLocale()).toBe('en-US')
    setBrowser(['en-US'])
    localStorage.setItem(LOCALE_STORAGE_KEY, 'ko-KR')
    expect(initialLocale()).toBe('ko-KR')
  })

  it('ignores a saved value that is not a language Penfii has', () => {
    setBrowser(['en-US'])
    localStorage.setItem(LOCALE_STORAGE_KEY, 'fr-FR')
    expect(initialLocale()).toBe('en-US')
  })

  it('uses the browser without a saved choice, and English without either', () => {
    setBrowser(['ko-KR'])
    expect(initialLocale()).toBe('ko-KR')
    setBrowser(['de-DE'])
    expect(initialLocale()).toBe('en-US')
  })

  it('survives storage that throws', () => {
    setBrowser(['ko-KR'])
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked')
    })
    expect(initialLocale()).toBe('ko-KR')
  })
})

function Probe() {
  const { locale } = useI18n()
  return <p data-testid="locale">{locale}</p>
}

describe('I18nProvider', () => {
  it('does not save a language it only detected', () => {
    setBrowser(['en-US'])
    render(
      <I18nProvider>
        <Probe />
      </I18nProvider>,
    )
    expect(screen.getByTestId('locale').textContent).toBe('en-US')
    expect(localStorage.getItem(LOCALE_STORAGE_KEY)).toBeNull()
    expect(document.documentElement.lang).toBe('en-US')
  })

  it('saves the choice made with the toggle, and the next visit keeps it', () => {
    setBrowser(['en-US'])
    const { unmount } = render(
      <I18nProvider>
        <Probe />
        <LocaleToggle />
      </I18nProvider>,
    )
    act(() => screen.getByRole('button').click())
    expect(screen.getByTestId('locale').textContent).toBe('ko-KR')
    expect(localStorage.getItem(LOCALE_STORAGE_KEY)).toBe('ko-KR')
    unmount()
    render(
      <I18nProvider>
        <Probe />
      </I18nProvider>,
    )
    expect(screen.getByTestId('locale').textContent).toBe('ko-KR')
  })
})

describe('I18nProvider when the texts cannot be fetched', () => {
  it('says so with a reload button instead of staying blank', async () => {
    vi.resetModules()
    vi.doMock('./catalog', () => ({ loadedCatalog: () => undefined, loadCatalog: () => Promise.reject(new Error('offline')) }))
    const { I18nProvider: Provider } = await import('./I18nProvider')
    render(<Provider><p>app</p></Provider>)
    expect((await screen.findByRole('alert')).textContent).toContain('Reload')
    expect(screen.queryByText('app')).toBeNull()
    vi.doUnmock('./catalog')
    vi.resetModules()
  })
})
