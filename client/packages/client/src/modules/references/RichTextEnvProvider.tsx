import { useMemo, type ReactNode } from 'react'
import { useNavigate } from 'react-router'
import type { Space } from '@penfii/contracts'
import { api } from '../../api'
import { RichTextEnvContext, type RichTextEnv } from '../../richtext/env'
import { ReferenceResolver } from '../docs/referenceCache'

/** Gives chat bodies, comments and quotes the names and link titles they show (one resolver per workspace visit). */
export function RichTextEnvProvider({ workspaceId, space, children }: { workspaceId: string; space: Space | undefined; children: ReactNode }) {
  const resolver = useMemo(() => new ReferenceResolver((items) => api.resolveReferences(workspaceId, items)), [workspaceId])
  const navigate = useNavigate()
  const value = useMemo<RichTextEnv>(() => ({ workspaceId, resolver, space, navigate: (to) => void navigate(to) }), [workspaceId, resolver, space, navigate])
  return <RichTextEnvContext.Provider value={value}>{children}</RichTextEnvContext.Provider>
}
