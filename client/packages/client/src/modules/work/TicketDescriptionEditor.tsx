import { useEffect, useMemo, useRef, useState } from 'react'
import { SuggestionMenuController, useCreateBlockNote } from '@blocknote/react'
import type { DocContent } from '@penfii/contracts'
import { useI18n } from '../../i18n/I18nProvider'
import { api } from '../../api'
import { BlockNoteSurface, blockNoteDictionary } from '../docs/BlockNoteSurface'
import { toBlocks, toContent } from '../docs/draftModel'
import { docSchema, type DocReferenceKind } from '../docs/schema'
import { DocContext, useOpenReference, type DocEditorContext } from '../docs/docContext'
import { ReferencePicker } from '../docs/ReferencePicker'
import { ReferenceResolver } from '../docs/referenceCache'
import { mentionMenuItems, slashMenuItems } from '../docs/suggestionItems'

function useTicketDocContext(workspaceId: string): DocEditorContext {
  const resolver = useMemo(() => new ReferenceResolver((items) => api.resolveReferences(workspaceId, items)), [workspaceId])
  const open = useOpenReference(workspaceId)
  return useMemo(() => ({ workspaceId, resolver, open }), [workspaceId, resolver, open])
}

function useDocModelEditor(content: DocContent) {
  const { locale } = useI18n()
  return useCreateBlockNote({ schema: docSchema, dictionary: blockNoteDictionary(locale), initialContent: toBlocks(content) as any }, [locale])
}

/**
 * A Ticket description on the Docs editor model, without files (a Ticket holds files as attachments). It starts
 * from `content` once; to start over, mount it again with a new `key`.
 */
export function TicketDescriptionEditor({ content, workspaceId, projectId, onChange, editable, onCancel, onSave }: {
  content: DocContent
  workspaceId: string
  projectId: string
  onChange: (content: DocContent) => void
  editable: boolean
  onCancel: () => void
  onSave: () => void
}) {
  const { t } = useI18n()
  const context = useTicketDocContext(workspaceId)
  const [picker, setPicker] = useState<{ kind: DocReferenceKind; insert: (id: string) => void } | null>(null)
  const editor = useDocModelEditor(content)
  const changeRef = useRef(onChange)
  changeRef.current = onChange
  useEffect(() => {
    editor.focus()
    return editor.onChange(() => changeRef.current(toContent(editor.document)))
  }, [editor])
  return (
    <DocContext.Provider value={context}>
      <div className="ticket-description-blocknote" onKeyDown={(event) => {
        // A key an open menu or toolbar of the editor has already taken (Escape closing the `/` or `@` menu) is not
        // the editor's own: it must not end the edit and drop what was typed.
        if (event.defaultPrevented) return
        if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); onCancel() }
        else if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !event.nativeEvent.isComposing) { event.preventDefault(); onSave() }
      }}>
        <BlockNoteSurface editor={editor} editable={editable}>
          {editable ? <SuggestionMenuController triggerCharacter="/" getItems={async (query) => slashMenuItems(editor, query, { files: false, t, pickReference: (kind, insert) => setPicker({ kind, insert }) })} /> : null}
          {editable ? <SuggestionMenuController triggerCharacter="@" getItems={(query) => mentionMenuItems(editor, workspaceId, query)} /> : null}
        </BlockNoteSurface>
      </div>
      {/* Outside the key handler above: Escape in the picker closes the picker, not the edit. */}
      {picker ? <ReferencePicker kind={picker.kind} workspaceId={workspaceId} projectId={projectId} onPick={(id) => { picker.insert(id); setPicker(null) }} onClose={() => { setPicker(null); editor.focus() }} /> : null}
    </DocContext.Provider>
  )
}

/** The saved description, rendered by the same editor read-only so it looks as it did while it was written. */
export function TicketDescriptionRead({ content, workspaceId }: { content: DocContent; workspaceId: string }) {
  const context = useTicketDocContext(workspaceId)
  const editor = useDocModelEditor(content)
  return <DocContext.Provider value={context}><div className="ticket-description-blocknote"><BlockNoteSurface editor={editor} editable={false} /></div></DocContext.Provider>
}
