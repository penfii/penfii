/**
 * A new input session of a compose box: a time-ordered UUID (version 7). The server keeps the newest session it has seen
 * for a box and refuses saves of older ones, so a later session must sort after an earlier one, also within the same
 * millisecond (a counter in the random part) and also when two tabs make theirs.
 */
let lastMs = 0
let counter = 0

export function newInputSession(now: number = Date.now()): string {
  let ms = now
  if (ms <= lastMs) {
    // The clock did not move on (or went back): keep counting from the last one, so this session sorts after it.
    ms = lastMs
    counter += 1
    if (counter > 0xfff) {
      ms += 1
      counter = 0
    }
  } else {
    counter = 0
  }
  lastMs = ms
  const random = new Uint8Array(8)
  crypto.getRandomValues(random)
  const hex = (value: number, width: number) => value.toString(16).padStart(width, '0')
  const time = hex(ms, 12)
  const bytes = Array.from(random, (byte) => hex(byte, 2)).join('')
  // 48 bits of milliseconds, version 7, 12 bits of counter, variant 10, then random.
  const variant = hex((random[0]! & 0x3f) | 0x80, 2)
  return `${time.slice(0, 8)}-${time.slice(8)}-7${hex(counter, 3)}-${variant}${bytes.slice(2, 4)}-${bytes.slice(4, 16)}`
}
