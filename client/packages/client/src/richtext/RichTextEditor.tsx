import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react'
import type { RichTextDoc } from '@penfii/contracts'
import { EditorContent, useEditor } from '@tiptap/react'
import { useI18n } from '../i18n/I18nProvider'
import { editorToRichText, richTextToEditor, sameRichText } from './adapter'
import { editorExtensions } from './editorExtensions'
import { containsFence, fenceOpening } from './fence'
import { pastedLink } from './linkPaste'
import { parseRichText } from './parse'
import { toggleCodeBlock, toggleInlineCode } from './codeCommands'
import { TOOLBAR_SHORTCUTS, ariaShortcut, commandTitle } from './shortcuts'
import { isSafeHttpUrl } from './url'

function openFence(editor: NonNullable<ReturnType<typeof useEditor>>): boolean {
  return editor.chain().focus().command(({ tr, state }) => {
    const { $from } = state.selection
    if ($from.parent.type.name !== 'paragraph') return false
    const opening = fenceOpening($from.parent.textContent)
    if (!opening) return false
    const node = state.schema.nodes.codeBlock?.create({ language: opening.language })
    if (!node) return false
    tr.replaceWith($from.before(), $from.after(), node)
    return true
  }).run()
}

function markdownLike(text: string): boolean {
  return /(\*\*|~~|(^|[^*])\*[^*\n]+\*|`|^#{1,6} |^> |^[-*] |^\d+\. |\[[^\]]+\]\(https?:\/\/)/m.test(text)
}

export type RichTextEditorHandle = {
  insertText: (text: string) => void
  /** Puts the caret in the editor. Used when a writing surface becomes the active one. False when the view is not ready. */
  focus: () => boolean
}

export const RichTextEditor = forwardRef<RichTextEditorHandle, {
  value: RichTextDoc
  onChange: (value: RichTextDoc) => void
  onSend?: () => void
  label: string
  placeholder?: string
  baseline?: RichTextDoc | null
  onFiles?: (files: File[]) => void
  /** Puts the cursor at the end of the text when the editor appears. */
  autoFocus?: boolean
}>(function RichTextEditor({
  value,
  onChange,
  onSend,
  label,
  placeholder,
  baseline,
  onFiles,
  autoFocus = false,
}, ref) {
  const { t } = useI18n()
  const onChangeRef = useRef(onChange)
  const onSendRef = useRef(onSend)
  const baselineRef = useRef(baseline ?? null)
  const placeholderRef = useRef(placeholder ?? '')
  const onFilesRef = useRef(onFiles)
  const editorRef = useRef<ReturnType<typeof useEditor>>(null)
  const [linkOpen, setLinkOpen] = useState(false)
  const [linkHref, setLinkHref] = useState('')
  const [revision, setRevision] = useState(0)
  onChangeRef.current = onChange
  onSendRef.current = onSend
  baselineRef.current = baseline ?? null
  placeholderRef.current = placeholder ?? ''
  onFilesRef.current = onFiles

  const editor = useEditor({
    immediatelyRender: true,
    autofocus: autoFocus ? 'end' : false,
    extensions: editorExtensions(() => placeholderRef.current),
    content: richTextToEditor(value),
    editorProps: {
      attributes: {
        role: 'textbox',
        'aria-multiline': 'true',
        'aria-label': label,
      },
      handleKeyDown: (_view, event) => {
        if (event.key !== 'Enter' || event.shiftKey) return false
        if (event.isComposing || event.keyCode === 229) return true
        const current = editorRef.current
        if (!current) return false
        if (current.isActive('listItem') || current.isActive('codeBlock')) return false
        if (openFence(current)) {
          event.preventDefault()
          return true
        }
        if (!onSendRef.current) return false
        event.preventDefault()
        onSendRef.current()
        return true
      },
      handlePaste: (_view, event) => {
        const current = editorRef.current
        if (!current) return false
        const files = [...(event.clipboardData?.files ?? [])]
        if (files.length > 0 && onFilesRef.current) {
          event.preventDefault()
          onFilesRef.current(files)
          return true
        }
        const text = event.clipboardData?.getData('text/plain') ?? ''
        const html = event.clipboardData?.getData('text/html') ?? ''
        if (!current.state.selection.empty) {
          const href = pastedLink(text)
          if (href) {
            event.preventDefault()
            current.chain().focus().extendMarkRange('link').setLink({ href }).run()
            return true
          }
          const attempt = text.trim()
          if (attempt.length > 0 && !/\s/.test(attempt) && /^[a-z][a-z0-9+.-]*:/i.test(attempt)) {
            event.preventDefault()
            return true
          }
        }
        if (text.length === 0) return false
        if (html.trim().length > 0 && !containsFence(text)) return false
        if (!text.includes('\n') && !markdownLike(text) && !containsFence(text)) return false
        const imported = richTextToEditor(parseRichText(text))
        current.commands.insertContent(imported.content ?? [])
        return true
      },
      handleDrop: (_view, event) => {
        const files = [...(event.dataTransfer?.files ?? [])]
        if (files.length === 0 || !onFilesRef.current) return false
        event.preventDefault()
        onFilesRef.current(files)
        return true
      },
      handleDOMEvents: {
        dragover: (_view, event) => {
          if (![...(event.dataTransfer?.types ?? [])].includes('Files')) return false
          event.preventDefault()
          return true
        },
      },
    },
    onUpdate: ({ editor: current }) => {
      onChangeRef.current(editorToRichText(current.getJSON(), baselineRef.current))
    },
    onTransaction: () => setRevision((current) => current + 1),
  })
  editorRef.current = editor
  useImperativeHandle(ref, () => ({
    insertText(text: string) {
      editor?.chain().focus().insertContent(text).run()
    },
    focus() {
      // `commands` reads the view, and that throws until EditorContent has mounted it.
      if (!editor || editor.isDestroyed || !editor.view) return false
      editor.commands.focus('end')
      return true
    },
  }), [editor])

  useEffect(() => {
    if (!editor) return
    editor.setOptions({
      editorProps: {
        ...editor.options.editorProps,
        attributes: {
          ...editor.options.editorProps.attributes,
          'aria-label': label,
        },
      },
    })
  }, [editor, label])

  useEffect(() => {
    if (!editor) return
    const current = editorToRichText(editor.getJSON(), baseline ?? null)
    if (sameRichText(current, value)) return
    editor.commands.setContent(richTextToEditor(value), { emitUpdate: false })
  }, [baseline, editor, value])

  function openLink() {
    if (!editor) return
    const current = editor.getAttributes('link').href
    setLinkHref(typeof current === 'string' ? current : '')
    setLinkOpen(true)
    editor.chain().focus().run()
  }

  if (editor) {
    const storage = editor.storage as { linkShortcut?: { open: () => void } }
    if (storage.linkShortcut) storage.linkShortcut.open = openLink
  }

  function applyLink() {
    if (!editor) return
    const href = linkHref.trim()
    if (!isSafeHttpUrl(href)) return
    const chain = editor.chain().focus()
    if (editor.state.selection.empty) {
      chain.insertContent({ type: 'text', text: href, marks: [{ type: 'link', attrs: { href } }] }).run()
    } else {
      chain.extendMarkRange('link').setLink({ href }).run()
    }
    setLinkOpen(false)
  }

  const pressed = (name: string) => Boolean(editor?.isActive(name))
  return (
    <div className="rich-editor" data-revision={revision}>
      <div
        className="rich-toolbar"
        role="toolbar"
        aria-label={label}
        onMouseDown={(event) => {
          if (event.target instanceof Element && event.target.closest('button')) event.preventDefault()
        }}
      >
        <span className="toolbar-group">
        <button type="button" aria-label={t('composer.bold')} aria-pressed={pressed('bold')} aria-keyshortcuts={ariaShortcut(TOOLBAR_SHORTCUTS.bold)} title={commandTitle(t('composer.bold'), TOOLBAR_SHORTCUTS.bold)} onClick={() => editor?.chain().focus().toggleBold().run()}><span aria-hidden="true" className="g-bold">B</span></button>
        <button type="button" aria-label={t('composer.italic')} aria-pressed={pressed('italic')} aria-keyshortcuts={ariaShortcut(TOOLBAR_SHORTCUTS.italic)} title={commandTitle(t('composer.italic'), TOOLBAR_SHORTCUTS.italic)} onClick={() => editor?.chain().focus().toggleItalic().run()}><span aria-hidden="true" className="g-italic">I</span></button>
        <button type="button" aria-label={t('composer.strike')} aria-pressed={pressed('strike')} aria-keyshortcuts={ariaShortcut(TOOLBAR_SHORTCUTS.strike)} title={commandTitle(t('composer.strike'), TOOLBAR_SHORTCUTS.strike)} onClick={() => editor?.chain().focus().toggleStrike().run()}><span aria-hidden="true" className="g-strike">S</span></button>
        </span>
        <span className="toolbar-sep" aria-hidden="true" />
        <span className="toolbar-group">
        <button type="button" aria-label={t('composer.quote')} aria-pressed={pressed('blockquote')} aria-keyshortcuts={ariaShortcut(TOOLBAR_SHORTCUTS.blockquote)} title={commandTitle(t('composer.quote'), TOOLBAR_SHORTCUTS.blockquote)} onClick={() => editor?.chain().focus().toggleBlockquote().run()}><span aria-hidden="true" className="">❝</span></button>
        <button type="button" aria-label={t('composer.bullet')} aria-pressed={pressed('bulletList')} aria-keyshortcuts={ariaShortcut(TOOLBAR_SHORTCUTS.bulletList)} title={commandTitle(t('composer.bullet'), TOOLBAR_SHORTCUTS.bulletList)} onClick={() => editor?.chain().focus().toggleBulletList().run()}><span aria-hidden="true" className="">•≡</span></button>
        <button type="button" aria-label={t('composer.ordered')} aria-pressed={pressed('orderedList')} aria-keyshortcuts={ariaShortcut(TOOLBAR_SHORTCUTS.orderedList)} title={commandTitle(t('composer.ordered'), TOOLBAR_SHORTCUTS.orderedList)} onClick={() => editor?.chain().focus().toggleOrderedList().run()}><span aria-hidden="true" className="">1.</span></button>
        </span>
        <span className="toolbar-sep" aria-hidden="true" />
        <span className="toolbar-group">
        <button type="button" aria-label={t('composer.code')} aria-pressed={pressed('code')} aria-keyshortcuts={ariaShortcut(TOOLBAR_SHORTCUTS.code)} title={commandTitle(t('composer.code'), TOOLBAR_SHORTCUTS.code)} onClick={() => { if (editor) toggleInlineCode(editor) }}><span aria-hidden="true" className="">&lt;/&gt;</span></button>
        <button type="button" aria-label={t('composer.codeBlock')} aria-pressed={pressed('codeBlock')} aria-keyshortcuts={ariaShortcut(TOOLBAR_SHORTCUTS.codeBlock)} title={commandTitle(t('composer.codeBlock'), TOOLBAR_SHORTCUTS.codeBlock)} onClick={() => { if (editor) toggleCodeBlock(editor) }}><span aria-hidden="true">{'{ }'}</span></button>
        </span>
        <span className="toolbar-sep" aria-hidden="true" />
        <button
          type="button"
          aria-label={t('composer.link')}
          aria-pressed={pressed('link') || linkOpen}
          aria-keyshortcuts={ariaShortcut(TOOLBAR_SHORTCUTS.link)}
          title={commandTitle(t('composer.link'), TOOLBAR_SHORTCUTS.link)}
          onClick={() => {
            if (!editor) return
            if (editor.isActive('link') && !linkOpen) {
              editor.chain().focus().unsetLink().run()
              setLinkOpen(false)
              return
            }
            openLink()
          }}
        >
          <svg aria-hidden="true" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1" /></svg>
        </button>
        {linkOpen ? (
          <input
            className="rich-link"
            aria-label={t('composer.linkUrl')}
            value={linkHref}
            onChange={(event) => setLinkHref(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault()
                applyLink()
              }
            }}
          />
        ) : null}
      </div>
      <EditorContent editor={editor} />
    </div>
  )
})
