import { act } from '@testing-library/react'
import type { Editor } from '@tiptap/core'

/** The editor behind a rich text box (its contenteditable, found by role `textbox`): tests type into it through it. */
export function editorOf(textbox: HTMLElement): Editor {
  return (textbox as unknown as { editor: Editor }).editor
}

/** Types `text` at the caret, the way the editor takes input (one transaction, so what listens for `@` or for changes does). */
export function typeInto(textbox: HTMLElement, text: string): void {
  act(() => {
    editorOf(textbox).chain().focus().insertContent(text).run()
  })
}

/** Replaces everything in the box with `text`. */
export function replaceTextIn(textbox: HTMLElement, text: string): void {
  act(() => {
    editorOf(textbox).chain().focus().selectAll().insertContent(text).run()
  })
}
