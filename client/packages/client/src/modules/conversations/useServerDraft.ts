import { useEffect, useRef } from 'react'
import type { RichTextDoc } from '@penfii/contracts'
import { isBlankDoc } from '../../richtext'
import { createDraftController, type DraftController, type DraftKind } from './draftController'

/**
 * Keeps what is typed in a compose box as the person's Draft on the server (see `createDraftController` for how a Draft
 * is tied to an input session so that a late save cannot bring a sent message back). It loads the stored text only into
 * an empty box, and text typed before `contextId` was known is saved as soon as it is.
 */
export function useServerDraft(kind: DraftKind, contextId: string | null, doc: RichTextDoc, setDoc: (update: (current: RichTextDoc) => RichTextDoc) => void) {
  const controller = useRef<DraftController | null>(null)
  const setDocRef = useRef(setDoc)
  setDocRef.current = setDoc
  const docRef = useRef(doc)
  docRef.current = doc
  /** The context of the previous setup, to tell a box whose drafts just opened from a switch to another box. */
  const lastContext = useRef<string | null>(null)

  useEffect(() => {
    const openedLate = lastContext.current === null
    lastContext.current = contextId
    if (!contextId) return
    const created = createDraftController({
      kind,
      contextId,
      fill: (body) => setDocRef.current((existing) => (isBlankDoc(existing) ? body : existing)),
    })
    controller.current = created
    // Drafts open only once the Space has loaded, and a person can type before that. That text is this box's: it is saved now
    // (and keeps a stored Draft from being loaded over it). Not on a switch from another context: until the page empties
    // the box, `doc` still holds the text of the box that was left.
    if (openedLate && !isBlankDoc(docRef.current)) created.update(docRef.current)
    return () => {
      // Before the next box is set up (an effect cleanup runs before the next effects): what this box holds is saved.
      created.dispose()
      if (controller.current === created) controller.current = null
    }
  }, [kind, contextId])

  useEffect(() => {
    controller.current?.update(doc)
  }, [doc])
}
