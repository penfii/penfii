import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError } from '@penfii/api-client'
import { api } from '../../api'
import { Dialog, Select, useConfirm } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { queryKeys } from '../../queryKeys'
import { errorText } from '../../ui/errorText'
import { useSession } from '../identity'
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
  canRemove = false,
  viewerIsMember = true,
  allowMemberInvites,
  canSetInvites = false,
  onChanged,
}: {
  open: boolean
  onClose: () => void
  workspaceId: string
  channelId: string
  canAdd: boolean
  /** The caller may make members managers or take the role back; the server keeps the last manager. */
  canSetRoles?: boolean
  /** The caller may take other people out (the channel's manager and its Project's admin). */
  canRemove?: boolean
  /** The caller is in the channel. An admin who is not runs its people without reading it, and cannot put themself in. */
  viewerIsMember?: boolean
  /** The member-invitation switch as it is now, and whether the caller may change it. */
  allowMemberInvites?: boolean
  canSetInvites?: boolean
  /** The channel's own data (the switch, the counts) changed; read it again. */
  onChanged?: () => void
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const members = useQuery({ ...queries.channelMembers(channelId), enabled: open })
  const me = useSession()
  const { confirm, dialog: confirmDialog } = useConfirm()
  const [notice, setNotice] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const taken = new Set((members.data ?? []).map((member) => member.userId))
  // Running a channel is not a way into it: someone who cannot read it does not offer themself.
  if (!viewerIsMember && me.data?.id) taken.add(me.data.id)

  async function remove(member: { userId: string; displayName: string }) {
    setNotice(null)
    setError(null)
    if (!(await confirm({ title: t('channel.addPeople.remove'), body: t('channel.addPeople.removeConfirm', { name: member.displayName }), confirmLabel: t('channel.addPeople.remove'), tone: 'danger' }))) return
    try {
      await api.removeChannelMember(channelId, member.userId)
      await queryClient.invalidateQueries({ queryKey: queryKeys.channelMembers(channelId) })
      onChanged?.()
      setNotice(t('channel.addPeople.removed', { name: member.displayName }))
    } catch (failure) {
      await queryClient.invalidateQueries({ queryKey: queryKeys.channelMembers(channelId) })
      setError(isApiError(failure) && failure.code === 'last_manager' ? t('channel.addPeople.lastManager') : errorText(failure, t('error.generic'), t('error.dbUnavailable')))
    }
  }

  async function setInvites(allow: boolean) {
    setNotice(null)
    setError(null)
    try {
      await api.updateChannel(channelId, { allowMemberInvites: allow })
      await queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })
      onChanged?.()
      setNotice(t('channel.addPeople.invitesSaved'))
    } catch (failure) {
      setError(errorText(failure, t('error.generic'), t('error.dbUnavailable')))
    }
  }

  async function add(person: { id: string; displayName: string }) {
    setNotice(null)
    setError(null)
    try {
      await api.addChannelMember(channelId, person.id)
      await queryClient.invalidateQueries({ queryKey: queryKeys.channelMembers(channelId) })
      setNotice(t('channel.addPeople.added', { name: person.displayName }))
    } catch (failure) {
      setError(errorText(failure, t('error.generic'), t('error.dbUnavailable')))
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
      setError(isApiError(failure) && failure.code === 'last_manager' ? t('channel.addPeople.lastManager') : errorText(failure, t('error.generic'), t('error.dbUnavailable')))
    }
  }

  return (
    <Dialog open={open} title={t('channel.addPeople.title')} onClose={onClose} presentation="full">
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
            {canRemove && member.userId !== me.data?.id ? (
              <button type="button" className="link-button" aria-label={`${t('channel.addPeople.remove')}: ${member.displayName}`} onClick={() => void remove(member)}>
                {t('channel.addPeople.remove')}
              </button>
            ) : null}
          </li>
        ))}
      </ul>
      {canSetInvites ? (
        <label className="hint">
          <input type="checkbox" checked={allowMemberInvites === true} onChange={(event) => void setInvites(event.target.checked)} /> {t('channel.addPeople.invites')}
        </label>
      ) : null}
      {canAdd ? <PersonPicker workspaceId={workspaceId} taken={taken} label={t('channel.addPeople.search')} onPick={(person) => void add(person)} /> : null}
      {notice ? <p role="status">{notice}</p> : null}
      {error ? <p className="form-error" role="alert">{error}</p> : null}
      {confirmDialog}
    </Dialog>
  )
}
