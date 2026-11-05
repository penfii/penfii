import { useRef } from 'react'
import { useI18n } from '../../i18n/I18nProvider'
import { IconButton } from '../../design/ui'
import { PaperclipIcon } from '../../design/icons'

/** The attach button with its hidden multi-file input. */
export function AttachButton({ onFiles, disabled = false }: { onFiles: (files: File[]) => void; disabled?: boolean }) {
  const { t } = useI18n()
  const input = useRef<HTMLInputElement>(null)
  return (
    <>
      <IconButton label={t('composer.attach')} disabled={disabled} onClick={() => input.current?.click()}><PaperclipIcon /></IconButton>
      <input
        ref={input}
        type="file"
        multiple
        hidden
        disabled={disabled}
        onChange={(event) => {
          onFiles([...(event.target.files ?? [])])
          event.target.value = ''
        }}
      />
    </>
  )
}
