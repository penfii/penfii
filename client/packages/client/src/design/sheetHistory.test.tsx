import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { useState } from 'react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { I18nProvider } from '../i18n/I18nProvider'
import { DESKTOP, PHONE, setViewport } from '../test/viewport'
import { Sheet } from './Sheet'
import { Dialog } from './ui'

function Host() {
  const [open, setOpen] = useState(false)
  return (
    <I18nProvider>
      <button type="button" onClick={() => setOpen(true)}>opener</button>
      <Sheet open={open} title="Pick" onClose={() => setOpen(false)}><p>inside</p></Sheet>
    </I18nProvider>
  )
}

function Chain() {
  const [sheet, setSheet] = useState(false)
  const [sure, setSure] = useState(false)
  return (
    <I18nProvider>
      <button type="button" onClick={() => setSheet(true)}>opener</button>
      <Sheet open={sheet} title="Pick" onClose={() => setSheet(false)}>
        <button type="button" onClick={() => { setSheet(false); setSure(true) }}>delete</button>
      </Sheet>
      <Dialog open={sure} title="Sure?" onClose={() => setSure(false)}><p>really</p></Dialog>
    </I18nProvider>
  )
}

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 20)))
const sheetMarks = () => (window.history.state as { sheet?: string } | null)?.sheet

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(() => cleanup())

describe('Sheet and the Back button', () => {
  it('closes with Back first, and a second Back leaves the screen', async () => {
    setViewport(PHONE)
    window.history.pushState({ idx: 1 }, '', '/screen')
    const before = window.history.length
    render(<Host />)
    fireEvent.click(screen.getByText('opener'))
    expect(screen.getByRole('dialog')).toBeTruthy()
    expect(window.history.length).toBe(before + 1)
    window.history.back()
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(window.location.pathname).toBe('/screen')
  })

  it('steps off its own entry when closed from inside, so Back is not spent on it', async () => {
    setViewport(PHONE)
    window.history.pushState({ idx: 1 }, '', '/screen')
    render(<Host />)
    fireEvent.click(screen.getByText('opener'))
    expect(sheetMarks()).toBeTruthy()
    fireEvent.click(screen.getByText('Close', { selector: '.m-sheet-close' }))
    await settle()
    expect(sheetMarks()).toBeUndefined()
  })

  it('does not undo a move that closes it', async () => {
    setViewport(PHONE)
    window.history.pushState({ idx: 1 }, '', '/screen')
    render(<Host />)
    fireEvent.click(screen.getByText('opener'))
    window.history.pushState({ idx: 3 }, '', '/elsewhere')
    cleanup()
    await settle()
    expect(window.location.pathname).toBe('/elsewhere')
  })

  it('leaves history alone on a wide window', async () => {
    setViewport(DESKTOP)
    window.history.pushState({ idx: 1 }, '', '/screen')
    const before = window.history.length
    render(<Host />)
    fireEvent.click(screen.getByText('opener'))
    expect(window.history.length).toBe(before)
  })

  it('lets a confirmation opened by a sheet take over the sheet entry: one Back closes it, nothing is left behind', async () => {
    setViewport(PHONE)
    HTMLDialogElement.prototype.showModal = function showModal() { this.setAttribute('open', '') }
    HTMLDialogElement.prototype.close = function close() { this.removeAttribute('open') }
    window.history.pushState({ idx: 1 }, '', '/screen')
    render(<Chain />)
    fireEvent.click(screen.getByText('opener'))
    const entry = window.history.length
    fireEvent.click(screen.getByText('delete'))
    await settle()
    expect(window.history.length).toBe(entry)
    expect(document.querySelector('dialog[open]')).toBeTruthy()
    window.history.back()
    await waitFor(() => expect(document.querySelector('dialog[open]')).toBeNull())
    await settle()
    expect(window.location.pathname).toBe('/screen')
    expect(sheetMarks()).toBeUndefined()
  })

  it('keeps a dialog that opts out (the answer to a navigation the router holds) out of the history', () => {
    setViewport(PHONE)
    HTMLDialogElement.prototype.showModal = function showModal() { this.setAttribute('open', '') }
    window.history.pushState({ idx: 1 }, '', '/screen')
    const before = window.history.length
    render(<I18nProvider><Dialog open title="Leave?" onClose={() => undefined} closeWithBack={false}><p>x</p></Dialog></I18nProvider>)
    expect(window.history.length).toBe(before)
    expect(sheetMarks()).toBeUndefined()
  })
})
