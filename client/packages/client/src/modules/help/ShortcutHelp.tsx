import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react'
import { Dialog } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { shortcutLabel } from '../../richtext/shortcuts'
import './shortcutHelp.css'

export type ShortcutSection = 'general' | 'editor'
type Open = (section?: ShortcutSection) => void

const ShortcutHelpContext = createContext<Open | null>(null)

/** Opens the one shortcut-help dialog, optionally at a section. Outside the provider it does nothing. */
export function useShortcutHelp(): Open {
  return useContext(ShortcutHelpContext) ?? (() => undefined)
}

/** True when a typed `?` belongs to the text the person is writing. */
function typingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  return target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName) || target.closest('[contenteditable="true"]') !== null
}

const EDITOR_ROWS: Array<{ key: 'bold' | 'italic' | 'underline' | 'strike' | 'code' | 'link' | 'undo' | 'redo'; chord: string }> = [
  { key: 'bold', chord: 'Mod-b' },
  { key: 'italic', chord: 'Mod-i' },
  { key: 'underline', chord: 'Mod-u' },
  { key: 'strike', chord: 'Mod-Shift-s' },
  { key: 'code', chord: 'Mod-e' },
  { key: 'link', chord: 'Mod-k' },
  { key: 'undo', chord: 'Mod-z' },
  { key: 'redo', chord: 'Mod-Shift-z' },
]

export function ShortcutHelpProvider({ children }: { children: ReactNode }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const [section, setSection] = useState<ShortcutSection>('general')
  const editorHeading = useRef<HTMLHeadingElement>(null)
  const show = useCallback<Open>((next = 'general') => {
    setSection(next)
    setOpen(true)
  }, [])

  // `?` opens the help, but never while the person is typing or composing text.
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key !== '?' || event.ctrlKey || event.metaKey || event.altKey || event.isComposing) return
      if (typingTarget(event.target)) return
      event.preventDefault()
      show('general')
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [show])

  useEffect(() => {
    if (open && section === 'editor') window.setTimeout(() => editorHeading.current?.focus(), 0)
  }, [open, section])

  return (
    <ShortcutHelpContext.Provider value={show}>
      {children}
      <Dialog open={open} title={t('shortcuts.title')} onClose={() => setOpen(false)}>
        <section aria-labelledby="shortcuts-general">
          <h3 id="shortcuts-general">{t('shortcuts.section.general')}</h3>
          <ul className="shortcut-list">
            <li><span>{t('shortcuts.general.help')}</span><kbd>?</kbd></li>
            <li><span>{t('shortcuts.general.palette')}</span><kbd>{shortcutLabel('Mod-k')}</kbd></li>
          </ul>
        </section>
        <section aria-labelledby="shortcuts-editor">
          <h3 id="shortcuts-editor" ref={editorHeading} tabIndex={-1}>{t('shortcuts.section.editor')}</h3>
          <p className="hint">{t('shortcuts.editor.lead')}</p>
          <ul className="shortcut-list">
            {EDITOR_ROWS.map((row) => (
              <li key={row.key}><span>{t(`shortcuts.editor.${row.key}`)}</span><kbd>{shortcutLabel(row.chord)}</kbd></li>
            ))}
            <li><span>{t('shortcuts.editor.indent')}</span><kbd>Tab</kbd></li>
            <li><span>{t('shortcuts.editor.outdent')}</span><kbd>Shift+Tab</kbd></li>
            <li><span>{t('shortcuts.editor.slash')}</span><kbd>/</kbd></li>
            <li><span>{t('shortcuts.editor.mention')}</span><kbd>@</kbd></li>
          </ul>
        </section>
      </Dialog>
    </ShortcutHelpContext.Provider>
  )
}
