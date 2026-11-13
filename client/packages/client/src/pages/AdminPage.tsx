import { useParams } from 'react-router'
import { WorkspaceAdmin } from '../modules/admin/WorkspaceAdmin'
import { parseAdminSection } from '../modules/admin/adminSections'

/** Workspace administration (Settings > Workspace). One page per section; `:section` picks it. */
export function AdminPage() {
  const { workspaceId = '', section } = useParams()
  return (
    <div className="page-scroll">
      <WorkspaceAdmin workspaceId={workspaceId} section={parseAdminSection(section)} />
    </div>
  )
}
