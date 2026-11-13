import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import {
  BasicTextStyleButton,
  ColorStyleButton,
  CreateLinkButton,
  NestBlockButton,
  UnnestBlockButton,
  useBlockNoteEditor,
} from '@blocknote/react'
import { useI18n } from '../../i18n/I18nProvider'
import { useIsMobile } from '../../design/useIsMobile'

/** The formatting controls of a page, shared by the selection toolbar and the phone's format bar. */
export function FormatButtons() {
  return (
    <>
      <BasicTextStyleButton basicTextStyle="bold" />
      <BasicTextStyleButton basicTextStyle="italic" />
      <BasicTextStyleButton basicTextStyle="underline" />
      <BasicTextStyleButton basicTextStyle="strike" />
      <BasicTextStyleButton basicTextStyle="code" />
      <ColorStyleButton />
      <NestBlockButton />
      <UnnestBlockButton />
      <CreateLinkButton />
    </>
  )
}

/**
 * On a phone, the formatting controls in one 44px row at the bottom of the phone shell, for as long as the editor has
 * focus. The shell is as tall as the visible area (see useKeyboardViewport), so its bottom edge is the top of the
 * keyboard whatever the length of the page: a short page does not leave the row floating under its text. It is a
 * flex child of the shell, so the page above it simply gets shorter. Putting the keyboard away takes the row away.
 * Save is not repeated here: the app bar's state and button are the one place.
 */
export function PhoneFormatBar() {
  const { t } = useI18n()
  const mobile = useIsMobile()
  const editor = useBlockNoteEditor()
  const [focused, setFocused] = useState(false)
  const [shell, setShell] = useState<Element | null>(null)
  useEffect(() => {
    const root = editor.domElement?.closest('.doc-editor')
    if (!mobile || !root) return
    setShell(root.closest('.mobile-shell'))
    const on = () => setFocused(true)
    // Focus may move into a control of the editor's own (the colour picker opens a popover) and stay "in" the editing.
    const off = (event: Event) => {
      const next = (event as FocusEvent).relatedTarget as Element | null
      if (next && (root.contains(next) || next.closest('.doc-format-bar, .mantine-Popover-dropdown, .bn-popover-content'))) return
      setFocused(false)
    }
    root.addEventListener('focusin', on)
    root.addEventListener('focusout', off)
    return () => {
      root.removeEventListener('focusin', on)
      root.removeEventListener('focusout', off)
    }
  }, [editor, mobile])
  if (!mobile || !focused || !shell) return null
  // Pressing a button must not take the focus (and with it the keyboard) away from the text.
  return createPortal(
    <div className="doc-format-bar" role="toolbar" aria-label={t('docs.format')} onMouseDown={(event) => event.preventDefault()}>
      <FormatButtons />
    </div>,
    shell,
  )
}
