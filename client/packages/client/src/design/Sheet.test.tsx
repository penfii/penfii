import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { useState } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../i18n/I18nProvider'
import { Dialog } from './ui'
import { DESKTOP, PHONE, setViewport } from '../test/viewport'
import { Sheet, SheetRow } from './Sheet'

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(() => {
  cleanup()
  document.getElementById('root')?.remove()
})

function Host() {
  const [open, setOpen] = useState(false)
  return (
    <I18nProvider>
      <button type="button" onClick={() => setOpen(true)}>opener</button>
      <Sheet open={open} title="Pick" onClose={() => setOpen(false)}>
        <SheetRow onClick={() => undefined}>One</SheetRow>
        <SheetRow onClick={() => undefined}>Two</SheetRow>
      </Sheet>
    </I18nProvider>
  )
}

describe('Sheet', () => {
  it('moves focus in, keeps Tab inside, and gives focus back when it closes', () => {
    render(<Host />)
    const opener = screen.getByRole('button', { name: 'opener' })
    opener.focus()
    fireEvent.click(opener)
    const sheet = screen.getByRole('dialog', { name: 'Pick' })
    expect(sheet.getAttribute('aria-modal')).toBe('true')
    expect(sheet.contains(document.activeElement)).toBe(true)
    // Tab from the last control wraps to the first.
    const items = [...sheet.querySelectorAll<HTMLElement>('button')]
    items[items.length - 1].focus()
    fireEvent.keyDown(items[items.length - 1], { key: 'Tab' })
    expect(document.activeElement).toBe(items[0])
    fireEvent.keyDown(items[0], { key: 'Tab', shiftKey: true })
    expect(document.activeElement).toBe(items[items.length - 1])
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(document.activeElement).toBe(opener)
  })

  it('stops the page behind from scrolling or taking focus while open, and releases it after', () => {
    const root = document.createElement('div')
    root.id = 'root'
    document.body.append(root)
    render(<Host />, { container: root.appendChild(document.createElement('div')) })
    fireEvent.click(screen.getByRole('button', { name: 'opener', hidden: true }))
    expect(document.documentElement.classList.contains('m-scroll-locked')).toBe(true)
    expect(root.hasAttribute('inert')).toBe(true)
    fireEvent.click(document.querySelector('.m-sheet-close') as HTMLElement)
    expect(document.documentElement.classList.contains('m-scroll-locked')).toBe(false)
    expect(root.hasAttribute('inert')).toBe(false)
  })

  it('closes when the dimmed backdrop is tapped', () => {
    render(<Host />)
    fireEvent.click(screen.getByRole('button', { name: 'opener' }))
    fireEvent.click(document.querySelector('.m-sheet-backdrop') as HTMLElement)
    expect(screen.queryByRole('dialog')).toBeNull()
  })
})

describe('Dialog presentation', () => {
  function show(presentation?: 'sheet' | 'full', onClose = vi.fn()) {
    render(
      <I18nProvider>
        <Dialog open title="Ask" onClose={onClose} presentation={presentation}>
          <input aria-label="name" />
        </Dialog>
      </I18nProvider>,
    )
    return { onClose, dialog: document.querySelector('dialog') as HTMLDialogElement }
  }

  beforeEach(() => {
    // jsdom does not implement the modal dialog API.
    HTMLDialogElement.prototype.showModal = function showModal() { this.setAttribute('open', '') }
    HTMLDialogElement.prototype.close = function close() { this.removeAttribute('open') }
  })

  it('is a sheet unless the caller asks for a full screen, whatever it contains', () => {
    expect(show().dialog.dataset.presentation).toBe('sheet')
    cleanup()
    expect(show('full').dialog.dataset.presentation).toBe('full')
  })

  it('closes a sheet when its backdrop is tapped on a phone, but not a full-screen dialog', () => {
    setViewport(PHONE)
    const sheet = show('sheet')
    fireEvent.click(sheet.dialog)
    expect(sheet.onClose).toHaveBeenCalled()
    cleanup()
    const full = show('full')
    fireEvent.click(full.dialog)
    expect(full.onClose).not.toHaveBeenCalled()
  })

  it('leaves a click on the dialog alone on a desktop-sized window and locks the page only on a phone', () => {
    setViewport(DESKTOP)
    const desktop = show('sheet')
    fireEvent.click(desktop.dialog)
    expect(desktop.onClose).not.toHaveBeenCalled()
    expect(document.documentElement.classList.contains('m-scroll-locked')).toBe(false)
    cleanup()
    setViewport(PHONE)
    show('sheet')
    expect(document.documentElement.classList.contains('m-scroll-locked')).toBe(true)
  })
})
