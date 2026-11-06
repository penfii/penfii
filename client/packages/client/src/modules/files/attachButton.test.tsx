import { cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../../i18n/I18nProvider'
import { DESKTOP, PHONE, setViewport } from '../../test/viewport'
import { AttachButton } from './AttachButton'

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(cleanup)

function mount(onFiles = vi.fn()) {
  render(<I18nProvider><AttachButton onFiles={onFiles} /></I18nProvider>)
  return onFiles
}

const inputs = () => [...document.querySelectorAll<HTMLInputElement>('input[type="file"]')]

describe('AttachButton on a phone', () => {
  beforeEach(() => setViewport(PHONE))

  it('asks where the file comes from before opening a picker', () => {
    mount()
    const clicks = inputs().map((input) => vi.spyOn(input, 'click'))
    fireEvent.click(screen.getByRole('button', { name: 'Attach file' }))
    const sheet = screen.getByRole('dialog', { name: 'Attach file' })
    expect(within(sheet).getAllByRole('button').filter((button) => button.classList.contains('m-sheet-row'))).toHaveLength(3)
    expect(clicks.every((spy) => spy.mock.calls.length === 0)).toBe(true)
  })

  it('opens the photo picker for images and video, and the camera for a new shot', () => {
    mount()
    fireEvent.click(screen.getByRole('button', { name: 'Attach file' }))
    const media = inputs().find((input) => input.accept === 'image/*,video/*' && !input.hasAttribute('capture')) as HTMLInputElement
    const camera = inputs().find((input) => input.hasAttribute('capture')) as HTMLInputElement
    const mediaClick = vi.spyOn(media, 'click')
    const cameraClick = vi.spyOn(camera, 'click')
    fireEvent.click(screen.getByRole('button', { name: 'Photos and videos' }))
    expect(mediaClick).toHaveBeenCalled()
    expect(screen.queryByRole('dialog')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Attach file' }))
    fireEvent.click(screen.getByRole('button', { name: 'Take a photo' }))
    expect(cameraClick).toHaveBeenCalled()
  })

  it('hands the chosen files on whichever picker they came from', () => {
    const onFiles = mount()
    const file = new File(['x'], 'a.png', { type: 'image/png' })
    const camera = inputs().find((input) => input.hasAttribute('capture')) as HTMLInputElement
    fireEvent.change(camera, { target: { files: [file] } })
    expect(onFiles).toHaveBeenCalledWith([file])
  })
})

describe('AttachButton on a desktop-sized window', () => {
  beforeEach(() => setViewport(DESKTOP))

  it('opens the file picker straight away, with no sheet', () => {
    mount()
    const [only] = inputs()
    const click = vi.spyOn(only, 'click')
    fireEvent.click(screen.getByRole('button', { name: 'Attach file' }))
    expect(click).toHaveBeenCalled()
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(inputs()).toHaveLength(1)
  })
})
