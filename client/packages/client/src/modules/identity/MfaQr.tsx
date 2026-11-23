import { encode } from 'uqr'
import { useI18n } from '../../i18n/I18nProvider'

const BORDER = 4 // modules of quiet zone the QR standard asks for

/** The dark modules of a QR code for `text` as one SVG path (one 1x1 square per module), and the side length. */
export function qrPath(text: string): { path: string; size: number } {
  const { data, size } = encode(text, { ecc: 'M', border: 0 })
  let path = ''
  data.forEach((row, y) => {
    row.forEach((dark, x) => {
      if (dark) path += `M${x + BORDER} ${y + BORDER}h1v1h-1z`
    })
  })
  return { path, size: size + BORDER * 2 }
}

/**
 * A QR code of the authenticator link, drawn in the browser (nothing is sent anywhere). Kept in its own
 * file so the encoder is a separate chunk that only the two-step setup loads; the key and the link stay
 * on the page as the way in when a camera is not at hand.
 */
export default function MfaQr({ uri }: { uri: string }) {
  const { t } = useI18n()
  const { path, size } = qrPath(uri)
  return (
    <figure className="mfa-qr">
      <svg role="img" aria-label={t('mfa.qrLabel')} viewBox={`0 0 ${size} ${size}`} width={192} height={192} shapeRendering="crispEdges">
        <rect width={size} height={size} fill="#fff" />
        <path d={path} fill="#000" />
      </svg>
      <figcaption className="hint">{t('mfa.qrHint')}</figcaption>
    </figure>
  )
}
