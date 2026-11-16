import { createContext, useCallback, useContext } from 'react'
import { useNavigate } from 'react-router'
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

/** Opens what a reference points at: a Ticket in its Project's panel, a Doc page. */
export function useOpenReference(workspaceId: string): DocEditorContext['open'] {
  const navigate = useNavigate()
  return useCallback((kind, id, hints) => {
    const base = `/w/${workspaceId}`
    if (kind === 'TICKET' && hints.projectId) void navigate(`${base}/projects/${hints.projectId}/work?panel=ticket:${id}`)
    else if (kind === 'DOC') void navigate(`${base}/docs?doc=${id}`)
  }, [navigate, workspaceId])
}
