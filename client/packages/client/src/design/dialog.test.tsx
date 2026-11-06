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
