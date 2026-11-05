import { useParams } from 'react-router'
import { WorkspaceAdmin } from '../modules/admin/WorkspaceAdmin'
import { McpPolicySettings } from '../modules/identity'

/** Workspace administration (Settings > Workspace administration). */
export function AdminPage() {
  const { workspaceId = '' } = useParams()
  return (
    <div className="page-scroll">
      <WorkspaceAdmin workspaceId={workspaceId} />
      <McpPolicySettings workspaceId={workspaceId} />
    </div>
  )
}
