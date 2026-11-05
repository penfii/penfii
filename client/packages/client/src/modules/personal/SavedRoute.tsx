import { useParams } from 'react-router'
import { SavedPage } from './SavedPage'

export function SavedPageRoute() {
  const { workspaceId = '' } = useParams()
  return <SavedPage workspaceId={workspaceId} />
}
