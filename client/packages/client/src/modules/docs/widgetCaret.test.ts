// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
import { installWidgetCaret } from './widgetCaret'

function setup(composing = false) {
  const root = document.createElement('div')
  root.innerHTML = '<p><span class="ProseMirror-widget" contenteditable="false">⁠<span>Alice</span>⁠</span><img class="ProseMirror-separator"></p>'
  document.body.append(root)
  const widget = root.querySelector('.ProseMirror-widget')!
  const view = { dom: root, composing, hasFocus: () => true } as never
  const off = installWidgetCaret(() => view)
  const inside = () => document.getSelection()!.collapse(widget.firstChild, 1)
  return { root, widget, off, inside }
}

describe('installWidgetCaret', () => {
  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('moves a caret that sits inside a remote cursor flag to just after it before a key is handled', () => {
    const { root, widget, off, inside } = setup()
    inside()
    root.dispatchEvent(new KeyboardEvent('keydown', { key: 'Y', bubbles: true }))
    const sel = document.getSelection()!
    expect(sel.anchorNode).toBe(widget.parentNode)
    expect(sel.anchorOffset).toBe(1)
    off()
  })

  it('leaves the caret alone while a composition is open', () => {
    const { root, widget, off, inside } = setup(true)
    inside()
    root.dispatchEvent(new KeyboardEvent('keydown', { key: 'Process', bubbles: true }))
    expect(document.getSelection()!.anchorNode).toBe(widget.firstChild)
    off()
  })

  it('does nothing for a caret in ordinary text', () => {
    const { root, off } = setup()
    const text = document.createTextNode('abc')
    root.querySelector('p')!.append(text)
    document.getSelection()!.collapse(text, 2)
    root.dispatchEvent(new KeyboardEvent('keydown', { key: 'Y', bubbles: true }))
    expect(document.getSelection()!.anchorNode).toBe(text)
    off()
  })
})
