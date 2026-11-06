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

// The tests were written for a Korean browser; the locale tests below set their own.
Object.defineProperty(navigator, 'language', { value: 'ko-KR', configurable: true })
Object.defineProperty(navigator, 'languages', { value: ['ko-KR', 'ko'], configurable: true })

// findBy*/waitFor default to 1 s; a loaded CI runner (shared NAS, 2-core GitHub) can miss that without any bug.
configure({ asyncUtilTimeout: 5000 })

afterEach(() => {
  cleanup()
  localStorage.clear()
})
