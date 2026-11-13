import { cleanup, configure } from '@testing-library/react'
import { afterEach } from 'vitest'

// jsdom has no modal dialogs; an open attribute is all the tests need to see the content.
if (typeof HTMLDialogElement !== 'undefined' && typeof HTMLDialogElement.prototype.showModal !== 'function') {
  HTMLDialogElement.prototype.showModal = function showModal(this: HTMLDialogElement) {
    this.setAttribute('open', '')
  }
  HTMLDialogElement.prototype.close = function close(this: HTMLDialogElement) {
    this.removeAttribute('open')
  }
}

// jsdom does no layout: it has no `getClientRects` on a Range or a text node. ProseMirror asks for them when it scrolls the
// caret into view, which an editor does a moment after a transaction (so, depending on timing, after the test body ended).
// An empty list is what a browser says for something that takes no space, and ProseMirror copes with it.
for (const prototype of [Range.prototype, Text.prototype]) {
  if (typeof (prototype as { getClientRects?: unknown }).getClientRects !== 'function') {
    Object.defineProperty(prototype, 'getClientRects', { value: () => [] as unknown as DOMRectList, configurable: true })
  }
}
if (typeof Range.prototype.getBoundingClientRect !== 'function') {
  Object.defineProperty(Range.prototype, 'getBoundingClientRect', { value: () => new DOMRect(0, 0, 0, 0), configurable: true })
}

// The tests were written for a Korean browser; the locale tests below set their own.
Object.defineProperty(navigator, 'language', { value: 'ko-KR', configurable: true })
Object.defineProperty(navigator, 'languages', { value: ['ko-KR', 'ko'], configurable: true })

// findBy*/waitFor default to 1 s; a loaded CI runner (shared NAS, 2-core GitHub) can miss that without any bug.
configure({ asyncUtilTimeout: 5000 })

afterEach(() => {
  cleanup()
  localStorage.clear()
})
