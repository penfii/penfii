import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { DirectoryPerson } from '@penfii/contracts'
import { useNavigate } from 'react-router'
import { api } from '../api'
import { Avatar } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import { useSession } from '../modules/identity'
import { queryKeys } from '../queryKeys'
import { queries } from '../queries'

export function PersonRow({
  person,
  selected,
  selectable = true,
  onToggle,
  onOpen,
}: {
  person: DirectoryPerson
  selected: boolean
  selectable?: boolean
  onToggle: () => void
  onOpen: () => void
}) {
  const { t } = useI18n()
  const handle = person.username ? `@${person.username}` : person.email
  const affiliation = [person.organizationName, ...person.affiliation].filter((item) => item.length > 0).join(' · ')
  return (
    <div className="directory-person">
      <button
        type="button"
        className="directory-row"
        data-directory-type="person"
        data-directory-name={person.displayName}
        onClick={onOpen}
      >
        <Avatar name={person.displayName} />
        <span>
          <strong>{person.displayName}</strong>
          <small>{handle}</small>
          <small>{affiliation}</small>
        </span>
      </button>
      {selectable ? (
        <label className="directory-check">
          <input
            type="checkbox"
            checked={selected}
            aria-label={`${t('search.include')} ${person.displayName}`}
            onChange={onToggle}
          />
        </label>
      ) : null}
    </div>
  )
}

export function useStartDm(workspaceId: string, onNavigate?: () => void) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const session = useSession()
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  const create = useMutation({
    mutationFn: (userIds: string[]) => api.createDm(workspaceId, { userIds }),
    onSuccess: async (dm) => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })
      go(`/w/${workspaceId}/c/${dm.conversationId}`)
    },
  })

  function go(path: string) {
    onNavigate?.()
    void navigate(path)
  }

  async function open(ids: string[]) {
    const me = session.data?.id
    if (!me) return
    const userIds = [me, ...ids.filter((id) => id !== me)]
    if (userIds.length < 2) return
    const wanted = [...userIds].sort().join(',')
    const existing = space.data?.dms.find((dm) => [...dm.memberIds].sort().join(',') === wanted)
    if (existing) {
      go(`/w/${workspaceId}/c/${existing.conversationId}`)
      return
    }
    // The failure shows through `error`; the caller only waits for the attempt to end.
    await create.mutateAsync(userIds).catch(() => undefined)
  }

  return { open, go, busy: create.isPending, error: create.isError }
}
