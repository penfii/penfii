import { useRef, useState } from 'react'
import { useI18n } from '../../i18n/I18nProvider'
import { IconButton } from '../../design/ui'
import { PaperclipIcon } from '../../design/icons'
import { Sheet, SheetRow } from '../../design/Sheet'
import { useIsMobile } from '../../design/useIsMobile'

type Source = 'files' | 'media' | 'camera'

/** The attach button with its hidden multi-file input. On a phone it first asks where the file comes from. */
export function AttachButton({ onFiles, disabled = false }: { onFiles: (files: File[]) => void; disabled?: boolean }) {
  const { t } = useI18n()
  const mobile = useIsMobile()
  const [choosing, setChoosing] = useState(false)
  const files = useRef<HTMLInputElement>(null)
  const media = useRef<HTMLInputElement>(null)
  const camera = useRef<HTMLInputElement>(null)
  const refs: Record<Source, typeof files> = { files, media, camera }
  function pick(source: Source) {
    setChoosing(false)
    refs[source].current?.click()
  }
  function read(event: React.ChangeEvent<HTMLInputElement>) {
    onFiles([...(event.target.files ?? [])])
    event.target.value = ''
  }
  return (
    <>
      <IconButton label={t('composer.attach')} disabled={disabled} onClick={() => (mobile ? setChoosing(true) : files.current?.click())}><PaperclipIcon /></IconButton>
      <input ref={files} type="file" multiple hidden disabled={disabled} onChange={read} />
      {mobile ? (
        <>
          <input ref={media} type="file" multiple accept="image/*,video/*" hidden disabled={disabled} onChange={read} />
          <input ref={camera} type="file" accept="image/*,video/*" capture="environment" hidden disabled={disabled} onChange={read} />
          <Sheet open={choosing} title={t('composer.attach')} onClose={() => setChoosing(false)}>
            <SheetRow onClick={() => pick('media')}>{t('mobile.attach.media')}</SheetRow>
            <SheetRow onClick={() => pick('camera')}>{t('mobile.attach.camera')}</SheetRow>
            <SheetRow onClick={() => pick('files')}>{t('mobile.attach.files')}</SheetRow>
          </Sheet>
        </>
      ) : null}
    </>
  )
}
