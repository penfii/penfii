import { useQuery } from '@tanstack/react-query'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { scopeLabel, type ShareScope } from './ShareScopePicker'

/** The current share scope of a page as the person reads it: "Whole Workspace", "Alpha + Design"... */
export function useScopeLabel(workspaceId: string, scope: ShareScope): string {
  const { t } = useI18n()
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  return scopeLabel(scope, space.data, t)
}
