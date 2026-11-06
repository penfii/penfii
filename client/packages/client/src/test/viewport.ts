import { afterEach } from 'vitest'

/** A phone-sized window (the layout is a phone at 640px and below) and a desktop-sized one. */
export const PHONE = 390
export const DESKTOP = 1280

const original = Object.getOwnPropertyDescriptor(window, 'matchMedia')

/**
 * Makes `window.matchMedia` answer `(max-width: Npx)` and `(min-width: Npx)` for a window of `width`
 * pixels, as a browser would, so a component that asks `useIsMobile()` takes its phone or its desktop
 * path. jsdom has no `matchMedia`; without this every test runs the desktop path.
 * Call it in the test (or `beforeEach`); the original is put back after each test.
 */
export function setViewport(width: number): void {
  const matches = (query: string) => {
    const max = /\(max-width:\s*(\d+)px\)/.exec(query)
    const min = /\(min-width:\s*(\d+)px\)/.exec(query)
    if (max) return width <= Number(max[1])
    if (min) return width >= Number(min[1])
    return false
  }
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    writable: true,
    value: (query: string): MediaQueryList => ({
      matches: matches(query),
      media: query,
      onchange: null,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
      addListener: () => undefined,
      removeListener: () => undefined,
      dispatchEvent: () => false,
    }),
  })
}

/** Registers the clean-up once per test file that imports this helper. */
afterEach(() => {
  if (original) Object.defineProperty(window, 'matchMedia', original)
  else Reflect.deleteProperty(window, 'matchMedia')
})
