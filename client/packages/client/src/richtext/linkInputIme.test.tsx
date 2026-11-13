import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { RichTextEditor } from './RichTextEditor'
import { renderWithProviders } from '../test/renderWithProviders'

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(cleanup)

describe('The link address box and input methods', () => {
  it('applies the address on a real Enter but not on the Enter that picks an IME candidate', async () => {
    const onChange = vi.fn()
    renderWithProviders(<RichTextEditor label="Body" value={{ type: 'doc', content: [] }} onChange={onChange} />)
    fireEvent.click(await screen.findByRole('button', { name: 'Link' }))
    const input = await screen.findByRole('textbox', { name: 'Link URL' })
    fireEvent.change(input, { target: { value: 'https://example.com' } })

    fireEvent.keyDown(input, { key: 'Enter', isComposing: true })
    fireEvent.keyDown(input, { key: 'Enter', keyCode: 229 })
    expect(screen.queryByRole('textbox', { name: 'Link URL' })).not.toBeNull()

    fireEvent.keyDown(input, { key: 'Enter', keyCode: 13 })
    await waitFor(() => expect(screen.queryByRole('textbox', { name: 'Link URL' })).toBeNull())
  })
})
