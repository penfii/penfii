import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError } from '@penfii/api-client'
import { api } from '../../api'
import { Dialog, Select } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { queryKeys } from '../../queryKeys'
import { errorText } from '../../ui/errorText'
import { PersonPicker } from './PersonPicker'

/**
 * The people of a channel and a way to put more in, the same picker that adds people to a Team. The
 * server decides who may do this and who may be put in (in a Project's channel, only the Project's
 * people), and says why when it refuses.
 */
export function ChannelMembersDialog({
  open,
  onClose,
  workspaceId,
  channelId,
  canAdd,
  canSetRoles = false,
}: {
  open: boolean
  onClose: () => void
  workspaceId: string
  channelId: string
  canAdd: boolean
  /** The caller may make members managers or take the role back; the server keeps the last manager. */
  canSetRoles?: boolean
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const members = useQuery({ ...queries.channelMembers(channelId), enabled: open })
  const [notice, setNotice] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const taken = new Set((members.data ?? []).map((member) => member.userId))

  async function add(person: { id: string; displayName: string }) {
    setNotice(null)
    setError(null)
    try {
      await api.addChannelMember(channelId, person.id)
      await queryClient.invalidateQueries({ queryKey: queryKeys.channelMembers(channelId) })
      setNotice(t('channel.addPeople.added', { name: person.displayName }))
    } catch (failure) {
      setError(errorText(failure, t('error.generic')))
    }
  }

  async function setRole(member: { userId: string; displayName: string }, role: 'MANAGER' | 'MEMBER') {
    setNotice(null)
    setError(null)
    try {
      await api.setChannelMemberRole(channelId, member.userId, role)
      // The caller's own buttons follow their own role, so the Space is read again too.
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.channelMembers(channelId) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) }),
      ])
      setNotice(t('channel.addPeople.roleSet', { name: member.displayName }))
    } catch (failure) {
      await queryClient.invalidateQueries({ queryKey: queryKeys.channelMembers(channelId) })
      setError(isApiError(failure) && failure.code === 'last_manager' ? t('channel.addPeople.lastManager') : errorText(failure, t('error.generic')))
    }
  }

  return (
    <Dialog open={open} title={t('channel.addPeople.title')} onClose={onClose}>
      <p className="hint">{t('channel.addPeople.hint')}</p>
      <h3>{t('channel.addPeople.members')}</h3>
      <ul className="directory-list" aria-label={t('channel.addPeople.members')}>
        {(members.data ?? []).map((member) => (
          <li key={member.userId} className="channel-member-row">
            <strong>{member.displayName}</strong>
            {canSetRoles ? (
              <Select
                aria-label={`${member.displayName}: ${t('channel.addPeople.role')}`}
                value={member.role}
                onChange={(event) => void setRole(member, event.target.value as 'MANAGER' | 'MEMBER')}
              >
                <option value="MANAGER">{t('channel.addPeople.manager')}</option>
                <option value="MEMBER">{t('channel.addPeople.member')}</option>
              </Select>
            ) : member.role === 'MANAGER' ? <small className="hint"> · {t('channel.addPeople.manager')}</small> : null}
          </li>
        ))}
      </ul>
      {canAdd ? <PersonPicker workspaceId={workspaceId} taken={taken} label={t('channel.addPeople.search')} onPick={(person) => void add(person)} /> : null}
      {notice ? <p role="status">{notice}</p> : null}
      {error ? <p className="form-error" role="alert">{error}</p> : null}
    </Dialog>
  )
}
