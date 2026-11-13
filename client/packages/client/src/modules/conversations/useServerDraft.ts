import { useEffect, useRef } from 'react'
import type { RichTextDoc } from '@penfii/contracts'
import { isBlankDoc } from '../../richtext'
import { createDraftController, type DraftController, type DraftKind } from './draftController'

/**
 * Keeps what is typed in a compose box as the person's Draft on the server (see `createDraftController` for how a Draft
 * is tied to an input session so that a late save cannot bring a sent message back). It loads the stored text only into
 * an empty box.
 */
export function useServerDraft(kind: DraftKind, contextId: string | null, doc: RichTextDoc, setDoc: (update: (current: RichTextDoc) => RichTextDoc) => void) {
  const controller = useRef<DraftController | null>(null)
  const setDocRef = useRef(setDoc)
  setDocRef.current = setDoc

  useEffect(() => {
    if (!contextId) return
    const created = createDraftController({
      kind,
      contextId,
      fill: (body) => setDocRef.current((existing) => (isBlankDoc(existing) ? body : existing)),
    })
    controller.current = created
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
