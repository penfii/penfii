import { createContext, useContext } from 'react'
import type { ReferenceKind } from '@penfii/contracts'
import type { ReferenceResolver } from './referenceCache'

export type DocEditorContext = {
  workspaceId: string
  resolver: ReferenceResolver | null
  /** Opens what a reference or mention points at (Ticket panel, Doc page). */
  open: (kind: ReferenceKind, id: string, hints: { projectId?: string }) => void
}

const empty: DocEditorContext = { workspaceId: '', resolver: null, open: () => undefined }

export const DocContext = createContext<DocEditorContext>(empty)

export function useDocContext(): DocEditorContext {
  return useContext(DocContext)
}
