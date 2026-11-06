import { useQuery } from '@tanstack/react-query'
import { useI18n } from '../i18n/I18nProvider'
import { queries } from '../queries'
import type { TitleSpec } from './routes'

/** The name shown on a pushed screen's bar: what `resolveScreen` said it is, looked up by name. */
export function useScreenTitle(workspaceId: string, spec: TitleSpec | null, fallback: string): string {
  const { t } = useI18n()
  const space = useQuery({ ...queries.space(workspaceId), enabled: Boolean(workspaceId) && (spec?.kind === 'project' || spec?.kind === 'conversation') })
  const docs = useQuery({ ...queries.docTree(workspaceId, spec?.kind === 'doc' ? spec.projectId : null), enabled: Boolean(workspaceId) && spec?.kind === 'doc' })
  const planning = useQuery({ ...queries.projectPlanning(spec?.kind === 'sprint' ? spec.projectId : ''), enabled: spec?.kind === 'sprint' })
  if (!spec) return fallback
  switch (spec.kind) {
    case 'key':
      return t(spec.key)
    case 'project':
      return space.data?.projects.find((item) => item.id === spec.id)?.name ?? fallback
    case 'conversation': {
      const channel = space.data?.channels.find((item) => item.conversationId === spec.id)
      if (channel) return `# ${channel.name}`
      const dm = space.data?.dms.find((item) => item.conversationId === spec.id)
      return dm?.peers.map((peer) => peer.displayName).join(', ') || fallback
    }
    case 'doc':
      return docs.data?.find((item) => item.id === spec.id)?.title ?? fallback
    case 'sprint':
      return planning.data?.sprints.find((item) => item.id === spec.id)?.name ?? fallback
  }
}
