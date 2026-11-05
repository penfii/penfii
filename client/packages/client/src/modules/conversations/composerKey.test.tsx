import type { RichTextDoc } from '@penfii/contracts'
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../../i18n/I18nProvider'
import { QueryWrapper } from '../../test/queryWrapper'
import { Composer } from './Composer'

function paragraph(text: string): RichTextDoc {
  return { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text }] }] }
}

describe('composer enter key', () => {
  it('does not send when Enter is pressed during IME composition', () => {
    const onSend = vi.fn()
    render(
      <QueryWrapper><I18nProvider>
        <Composer value={paragraph('안녕')} onChange={() => undefined} onSend={onSend} messageBodyMaxBytes={10_000} busy={false} label="message" workspaceId="workspace" />
      </I18nProvider></QueryWrapper>,
    )
    const textbox = screen.getByRole('textbox')
    fireEvent.keyDown(textbox, { key: 'Enter', shiftKey: false, isComposing: true })
    expect(onSend).not.toHaveBeenCalled()

    const native = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })
    Object.defineProperty(native, 'isComposing', { get: () => false })
    Object.defineProperty(native, 'keyCode', { get: () => 229 })
    textbox.dispatchEvent(native)
    expect(onSend).not.toHaveBeenCalled()
  })

  it('sends on Enter when the field is not composing, and not on Shift+Enter', () => {
    const onSend = vi.fn()
    render(
      <QueryWrapper><I18nProvider>
        <Composer value={paragraph('hello')} onChange={() => undefined} onSend={onSend} messageBodyMaxBytes={10_000} busy={false} label="message" workspaceId="workspace" />
      </I18nProvider></QueryWrapper>,
    )
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter', shiftKey: true, isComposing: false, keyCode: 13 })
    expect(onSend).not.toHaveBeenCalled()
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter', shiftKey: false, isComposing: false, keyCode: 13 })
    expect(onSend).toHaveBeenCalledTimes(1)
  })
})
