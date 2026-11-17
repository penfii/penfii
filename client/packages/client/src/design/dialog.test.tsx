import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { useState } from 'react'
import { afterEach, describe, expect, it } from 'vitest'
import { I18nProvider } from '../i18n/I18nProvider'
import { Dialog } from './ui'

// jsdom has no <dialog> modal behaviour: model what the browser does when a modal dialog is closed.
const closed: string[] = []
HTMLDialogElement.prototype.showModal = function showModal() { this.setAttribute('open', '') }
HTMLDialogElement.prototype.close = function close() { this.removeAttribute('open'); closed.push('close') }

function Host() {
  const [open, setOpen] = useState(false)
  return (
    <I18nProvider>
      <button type="button" onClick={() => setOpen(true)}>open</button>
      {open ? <Dialog open title="T" onClose={() => setOpen(false)}><p>body</p></Dialog> : null}
    </I18nProvider>
  )
}

afterEach(() => { cleanup(); closed.length = 0 })

describe('Dialog mounted on demand', () => {
  it('is simply removed when its owner closes it', () => {
    render(<Host />)
    fireEvent.click(screen.getByText('open'))
    expect(screen.getByText('body')).toBeTruthy()
    fireEvent.click(screen.getAllByRole('button', { name: /close|닫기/i })[0] as Element)
    expect(screen.queryByText('body')).toBeNull()
    expect(closed).toEqual([])
  })

  it('gives focus back to the button that opened it when it is removed while open', async () => {
    render(<Host />)
    const trigger = screen.getByText('open')
    trigger.focus()
    fireEvent.click(trigger)
    fireEvent.click(screen.getAllByRole('button', { name: /close|닫기/i })[0] as Element)
    await new Promise((resolve) => setTimeout(resolve, 5))
    expect(document.activeElement).toBe(trigger)
  })
})

function Nested({ onOuterClose }: { onOuterClose: () => void }) {
  const [inner, setInner] = useState(false)
  return (
    <I18nProvider>
      <Dialog open title="Outer" onClose={onOuterClose}>
        <button type="button" onClick={() => setInner(true)}>ask</button>
        <Dialog open={inner} title="Inner" onClose={() => setInner(false)}><p>inner body</p></Dialog>
      </Dialog>
    </I18nProvider>
  )
}

describe('a dialog opened from inside another', () => {
  it('closes alone: its close and Escape (cancel) do not close the dialog around it', () => {
    let outerClosed = 0
    render(<Nested onOuterClose={() => { outerClosed += 1 }} />)
    fireEvent.click(screen.getByText('ask'))
    const inner = screen.getByText('inner body').closest('dialog') as HTMLDialogElement
    // The browser's Escape: a cancel event on the inner dialog.
    fireEvent(inner, new Event('cancel', { cancelable: true }))
    expect(outerClosed).toBe(0)
    // The inner one closing (answered, or closed by its owner) fires close on it.
    fireEvent(inner, new Event('close'))
    expect(outerClosed).toBe(0)
    // The outer one's own events still close it.
    const outer = screen.getByText('ask').closest('dialog') as HTMLDialogElement
    fireEvent(outer, new Event('cancel', { cancelable: true }))
    expect(outerClosed).toBe(1)
  })
})
