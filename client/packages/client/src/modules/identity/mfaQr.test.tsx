import { describe, expect, it } from 'vitest'
import jsQR from 'jsqr'
import { qrPath } from './MfaQr'

/** Paints the path of qrPath (one 1x1 square per dark module) into RGBA pixels, `scale` pixels per module. */
function rasterise(path: string, size: number, scale: number): Uint8ClampedArray {
  const side = size * scale
  const pixels = new Uint8ClampedArray(side * side * 4).fill(255)
  for (const match of path.matchAll(/M(\d+) (\d+)h1v1h-1z/g)) {
    const [mx, my] = [Number(match[1]), Number(match[2])]
    for (let y = my * scale; y < (my + 1) * scale; y++) {
      for (let x = mx * scale; x < (mx + 1) * scale; x++) {
        const at = (y * side + x) * 4
        pixels[at] = pixels[at + 1] = pixels[at + 2] = 0
      }
    }
  }
  return pixels
}

describe('QR code of the authenticator link', () => {
  it('decodes back to the exact link, including a long secret and non-ASCII', () => {
    const uris = [
      'otpauth://totp/Penfii%3Aalice%40example.com?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=Penfii&algorithm=SHA1&digits=6&period=30',
      'otpauth://totp/Penfii%3A%ED%95%9C%EA%B8%80?secret=ABCDEFGHIJKLMNOP&issuer=Penfii',
    ]
    for (const uri of uris) {
      const { path, size } = qrPath(uri)
      const decoded = jsQR(rasterise(path, size, 6), size * 6, size * 6)
      expect(decoded?.data).toBe(uri)
    }
  })
})
