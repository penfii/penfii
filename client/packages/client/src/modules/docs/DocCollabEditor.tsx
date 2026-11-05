// Dormant: the page no longer mounts this editor. Wiki pages are edited as a local draft (DocEditor.tsx)
// and replaced on Save. This binds an editor to the Yjs/WebSocket co-editing layer, which the server
// still runs, so a live-collaboration document type can use it without rebuilding the layer.
import '@blocknote/mantine/style.css'
import './docEditor.css'
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import * as Y from 'yjs'
import { Awareness } from 'y-protocols/awareness'
import { MantineProvider } from '@mantine/core'
import { filterSuggestionItems } from '@blocknote/core'
import { withCollaboration } from '@blocknote/core/yjs'
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
import type { DocPage } from '@penfii/contracts'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { DocContext, type DocEditorContext } from './docContext'
import { useDocUploads } from './useDocUploads'
import { insertFiles } from './docUploads'
import { FileDropZone } from '../files/FileDropZone'
import { installCompositionGuard, readPendingInput } from './compositionGuard'
import { installWidgetCaret } from './widgetCaret'
import { DocCollabProvider, collabUrl, type CollabState } from './collabProvider'
import { ReferencePicker } from './ReferencePicker'
import { ReferenceResolver } from './referenceCache'
import { docSchema, type DocReferenceKind } from './schema'

export type EditorSession = {
  doc: Y.Doc
  awareness: Awareness
  provider: DocCollabProvider
}

type Props = {
  page: DocPage
  workspaceId: string
  user: { id: string; name: string }
  /** The caller shows status and banners from this; the editor itself only edits. */
  onState: (state: CollabState) => void
  /** The lineage changed under a stale document: the caller reloads the page and mounts a new editor. */
  onEpochReset: () => void
  open: DocEditorContext['open']
}

const PALETTE = ['#d9480f', '#1971c2', '#2f9e44', '#9c36b5', '#c2255c', '#0c8599', '#e8590c', '#5f3dc4']

function colorOf(id: string): string {
  let hash = 0
  for (const ch of id) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0
  return PALETTE[hash % PALETTE.length]!
}

/**
 * The BlockNote editor of one page, bound to the page's Yjs document.
 *
 * The page's text lives in the shared document while people edit; what REST returns is the last
 * materialization. A new session is made for every mount (and for every lineage reset), because a
 * document must never be reused across lineages: merging would duplicate the text.
 */
export default function DocEditor({ page, workspaceId, user, onState, onEpochReset, open }: Props) {
  const [session, setSession] = useState<EditorSession | null>(null)
  useEffect(() => {
    const doc = new Y.Doc()
    const awareness = new Awareness(doc)
    const provider = new DocCollabProvider({
      url: collabUrl(page.id, window.location),
      doc,
      awareness,
    })
    setSession({ doc, awareness, provider })
    provider.connect()
    return () => {
      provider.destroy()
      doc.destroy()
      setSession(null)
    }
  }, [page.id])
  if (!session) return null
  return <Body key={session.doc.guid} session={session} page={page} workspaceId={workspaceId} user={user} onState={onState} onEpochReset={onEpochReset} open={open} />
}

function Body({ session, page, workspaceId, user, onState, onEpochReset, open }: Props & { session: EditorSession }) {
  const { t, locale } = useI18n()
  const state = useSyncExternalStore(session.provider.subscribe, session.provider.getState)
  const resolver = useMemo(() => new ReferenceResolver((items) => api.resolveReferences(workspaceId, items)), [workspaceId])
  const [picker, setPicker] = useState<{ kind: DocReferenceKind; insert: (id: string) => void } | null>(null)
  // Typing before the first sync would build a page of its own beside the one on the server (which
  // seeds the content), so the editor opens for input once the server's state has arrived. After
  // that it stays editable through a disconnect.
  const [synced, setSynced] = useState(false)
  useEffect(() => {
    if (state.synced) setSynced(true)
  }, [state.synced])
  const readOnly = state.status === 'closed' || !synced

  useEffect(() => onState(state), [state, onState])
  useEffect(() => {
    if (state.status === 'epoch-reset') onEpochReset()
  }, [state.status, onEpochReset])
  // A reference may have changed what the viewer can read; look again when the tab comes back.
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === 'visible') resolver.refresh()
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => document.removeEventListener('visibilitychange', onVisible)
  }, [resolver])
  // Leaving with edits the server has not got would lose them: ask first.
  useEffect(() => {
    const guard = (event: BeforeUnloadEvent) => {
      if (session.provider.getState().unsynced > 0) {
        event.preventDefault()
        event.returnValue = ''
      }
    }
    window.addEventListener('beforeunload', guard)
    return () => window.removeEventListener('beforeunload', guard)
  }, [session.provider])

  const { uploads, extension: uploadsExtension, holder } = useDocUploads(workspaceId, (fileId) =>
    // Attaching now lets everyone in the room fetch the file before the next materialization.
    api.attachDocFile(page.id, fileId),
  )
  const editor = useCreateBlockNote(
    withCollaboration({
      schema: docSchema,
      dictionary: locale === 'ko-KR' ? koDictionary : enDictionary,
      collaboration: {
        fragment: session.doc.getXmlFragment('document-store'),
        provider: { awareness: session.awareness },
        user: { name: user.name, color: colorOf(user.id) },
        showCursorLabels: 'always',
      },
      uploadFile: uploads.uploadFile,
      extensions: [uploadsExtension],
    }),
    [session.doc.guid, locale, uploads],
  )
  holder.current = editor
  const editorRef = useRef(editor)
  editorRef.current = editor
  useEffect(() => {
    editor.isEditable = !readOnly
  }, [editor, readOnly])
  // Other people's edits wait while this person is composing (Korean, Japanese, ...) so the composition survives.
  useEffect(() => {
    const view = editor.prosemirrorView
    return view ? installCompositionGuard(view) : undefined
  }, [editor])

  // A key press the editor has not read yet must reach the shared document before a remote change is shown.
  useEffect(() => {
    session.provider.beforeRemoteUpdate = () => readPendingInput(editor.prosemirrorView)
    return () => {
      session.provider.beforeRemoteUpdate = null
    }
  }, [editor, session.provider])
  // A key press while the caret sits in someone's cursor flag would be dropped by the browser.
  useEffect(() => installWidgetCaret(() => editor.prosemirrorView), [editor])

  const context = useMemo<DocEditorContext>(() => ({ workspaceId, resolver, open }), [workspaceId, resolver, open])

  function slashItems(query: string): DefaultReactSuggestionItem[] {
    const base = getDefaultReactSlashMenuItems(editor).filter((item) => (item as { key?: string }).key !== 'emoji')
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
        <div className="doc-editor" data-doc-editor data-status={state.status}>
          <FileDropZone className="doc-drop" disabled={readOnly} onFiles={(dropped) => void insertFiles(editor, uploads, dropped)}>
          <BlockNoteView editor={editor} theme="light" slashMenu={false} formattingToolbar={false} editable={!readOnly}>
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
            <SuggestionMenuController triggerCharacter="/" getItems={async (query) => slashItems(query)} />
            <SuggestionMenuController triggerCharacter="@" getItems={mentionItems} />
          </BlockNoteView>
          </FileDropZone>
        </div>
      </MantineProvider>
      {picker ? (
        <ReferencePicker
          kind={picker.kind}
          workspaceId={workspaceId}
          projectId={page.projectId}
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
