import '@blocknote/mantine/style.css'
import './docEditor.css'
import { useShortcutHelp } from '../help/ShortcutHelp'
import { useEffect, useMemo, useRef, useState } from 'react'
import { MantineProvider } from '@mantine/core'
import { filterSuggestionItems } from '@blocknote/core'
import { en as enDictionary, ko as koDictionary } from '@blocknote/core/locales'
import {
  BasicTextStyleButton,
  ColorStyleButton,
  CreateLinkButton,
  FormattingToolbar,
  FormattingToolbarController,
  NestBlockButton,
  SuggestionMenuController,
  UnnestBlockButton,
  getDefaultReactSlashMenuItems,
  useCreateBlockNote,
  type DefaultReactSuggestionItem,
} from '@blocknote/react'
import { BlockNoteView } from '@blocknote/mantine'
import type { DocContent } from '@penfii/contracts'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { useDocUploads } from './useDocUploads'
import { insertFiles } from './docUploads'
import { FileDropZone } from '../files/FileDropZone'
import { DocContext, type DocEditorContext } from './docContext'
import { toBlocks, toContent } from './draftModel'
import { ReferencePicker } from './ReferencePicker'
import { ReferenceResolver } from './referenceCache'
import { docSchema, type DocReferenceKind } from './schema'

/** What the page asks of the editor: the draft as DocContent, and whether it differs from the start. */
export type EditorController = {
  getContent: () => DocContent
  isDirty: () => boolean
}

type Props = {
  docId: string
  projectId: string | null
  workspaceId: string
  /** The document the editor starts from (saved content, or a Version being restored). */
  content: DocContent
  editable: boolean
  /** Called with the controller when the editor is ready and with null when it goes away. */
  onController?: (controller: EditorController | null) => void
  onDirtyChange?: (dirty: boolean) => void
  /** The current draft, including the first paint. A new document uses this to stay in the browser. */
  onContentChange?: (content: DocContent) => void
  /** A saved page can hold files. A new document cannot, until the first save creates it. */
  files?: boolean
  /** Puts the caret in the page when this editor is the writing surface. */
  autoFocus?: boolean
  open: DocEditorContext['open']
}

/**
 * The BlockNote editor of one page. It holds a draft in the browser: nothing is sent while the
 * person types, and the page reads the result with `getContent()` when they press Save. It starts
 * from `content` once; to start over from other content, mount it again with a new `key`.
 */
export default function DocEditor({ docId, projectId, workspaceId, content, editable, onController, onDirtyChange, onContentChange, files = true, autoFocus = false, open }: Props) {
  const { t, locale } = useI18n()
  const openShortcutHelp = useShortcutHelp()
  const resolver = useMemo(() => new ReferenceResolver((items) => api.resolveReferences(workspaceId, items)), [workspaceId])
  const [picker, setPicker] = useState<{ kind: DocReferenceKind; insert: (id: string) => void } | null>(null)

  // The file is attached to the page at once: Save only accepts files that belong to the page.
  const { uploads, extension: uploadsExtension, holder } = useDocUploads(workspaceId, (fileId) => api.attachDocFile(docId, fileId))
  const editor = useCreateBlockNote(
    {
      schema: docSchema,
      dictionary: locale === 'ko-KR' ? koDictionary : enDictionary,
      initialContent: toBlocks(content) as any,
      uploadFile: files ? uploads.uploadFile : undefined,
      extensions: [uploadsExtension],
    },
    [docId, locale, uploads],
  )
  holder.current = editor
  useEffect(() => {
    if (!autoFocus || !editable) return
    editor.focus()
  }, [autoFocus, editable, editor])
  const editorRef = useRef(editor)
  editorRef.current = editor
  const onContentChangeRef = useRef(onContentChange)
  onContentChangeRef.current = onContentChange

  // The draft is dirty once the document differs from how the editor first showed it.
  const start = useRef<string | null>(null)
  const dirty = useRef(false)
  useEffect(() => {
    start.current = JSON.stringify(editor.document)
    dirty.current = false
    const controller: EditorController = {
      getContent: () => toContent(editor.document),
      isDirty: () => dirty.current,
    }
    onController?.(controller)
    onContentChangeRef.current?.(toContent(editor.document))
    const stop = editor.onChange(() => {
      const next = JSON.stringify(editor.document) !== start.current
      if (next !== dirty.current) {
        dirty.current = next
        onDirtyChange?.(next)
      }
      onContentChangeRef.current?.(toContent(editor.document))
    })
    return () => {
      stop?.()
      onController?.(null)
    }
  }, [editor, onController, onDirtyChange])

  // A reference may have changed what the viewer can read; look again when the tab comes back.
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === 'visible') resolver.refresh()
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => document.removeEventListener('visibilitychange', onVisible)
  }, [resolver])

  const context = useMemo<DocEditorContext>(() => ({ workspaceId, resolver, open }), [workspaceId, resolver, open])

  function slashItems(query: string): DefaultReactSuggestionItem[] {
    const fileKeys = new Set(['image', 'video', 'audio', 'file'])
    const base = getDefaultReactSlashMenuItems(editor).filter((item) => {
      const key = (item as { key?: string }).key
      if (key === 'emoji') return false
      return files || !key || !fileKeys.has(key)
    })
    const refItem = (kind: DocReferenceKind, title: string, aliases: string[]): DefaultReactSuggestionItem => ({
      title,
      aliases,
      group: t('docs.menu.references'),
      onItemClick: () => {
        setPicker({
          kind,
          insert: (id) => {
            editor.focus()
            editor.insertInlineContent([{ type: 'reference', props: { kind, id } }, ' '])
          },
        })
      },
    })
    return filterSuggestionItems(
      [
        ...base,
        refItem('TICKET', t('docs.menu.ticket'), ['ticket', 'task', '티켓', '작업']),
        refItem('DOC', t('docs.menu.doc'), ['doc', 'page', '문서', '페이지']),
      ],
      query,
    )
  }

  async function mentionItems(query: string): Promise<DefaultReactSuggestionItem[]> {
    const found = await api.searchDirectory(workspaceId, { q: query, types: 'person', limit: 8 })
    return found.results.flatMap((hit) => {
      const person = hit.type === 'person' ? hit.person : undefined
      if (!person) return []
      const item: DefaultReactSuggestionItem = {
        title: person.displayName,
        subtext: person.organizationName,
        onItemClick: () => editor.insertInlineContent([{ type: 'mention', props: { userId: person.id } }, ' ']),
      }
      return [item]
    })
  }

  return (
    <DocContext.Provider value={context}>
      <MantineProvider withGlobalClasses={false} withCssVariables={false}>
        {editable ? (
          <div className="doc-help-bar">
            <span className="hint">{t('editor.helpBar')}</span>
            <button type="button" className="btn btn-ghost" onClick={() => openShortcutHelp('editor')}>{t('shortcuts.open')}</button>
          </div>
        ) : null}
        <div className="doc-editor" data-doc-editor data-editable={editable}>
          <FileDropZone className="doc-drop" disabled={!editable || !files} onFiles={(dropped) => void insertFiles(editor, uploads, dropped)}>
          <BlockNoteView editor={editor} theme="light" slashMenu={false} formattingToolbar={false} editable={editable}>
            <FormattingToolbarController
              formattingToolbar={() => (
                <FormattingToolbar>
                  <BasicTextStyleButton basicTextStyle="bold" />
                  <BasicTextStyleButton basicTextStyle="italic" />
                  <BasicTextStyleButton basicTextStyle="underline" />
                  <BasicTextStyleButton basicTextStyle="strike" />
                  <BasicTextStyleButton basicTextStyle="code" />
                  <ColorStyleButton />
                  <NestBlockButton />
                  <UnnestBlockButton />
                  <CreateLinkButton />
                </FormattingToolbar>
              )}
            />
            {editable ? <SuggestionMenuController triggerCharacter="/" getItems={async (query) => slashItems(query)} /> : null}
            {editable ? <SuggestionMenuController triggerCharacter="@" getItems={mentionItems} /> : null}
          </BlockNoteView>
          </FileDropZone>
        </div>
      </MantineProvider>
      {picker ? (
        <ReferencePicker
          kind={picker.kind}
          workspaceId={workspaceId}
          projectId={projectId}
          onPick={(id) => {
            picker.insert(id)
            setPicker(null)
          }}
          onClose={() => {
            setPicker(null)
            editorRef.current.focus()
          }}
        />
      ) : null}
    </DocContext.Provider>
  )
}
