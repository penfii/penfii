import { useState } from 'react'
import { isApiError } from '@penfii/api-client'
import { api } from '../../api'
import { Button, useConfirm } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { errorText } from '../../ui/errorText'

/**
 * A member leaves a private channel after confirming. Only the person themselves can do it; the
 * server keeps the last manager in and says so, and the page decides where to go once it is done.
 */
export function LeaveChannelButton({
  channelId,
  channelName,
  userId,
  onLeft,
  onChooseManager,
}: {
  channelId: string
  channelName: string
  userId: string
  onLeft: () => void
  /** Opens the member list, where another person can be made a manager. */
  onChooseManager?: () => void
}) {
  const { t } = useI18n()
  const { confirm, dialog } = useConfirm()
  const [error, setError] = useState<string | null>(null)
  const [lastManager, setLastManager] = useState(false)

  async function leave() {
    setError(null)
    setLastManager(false)
    if (!(await confirm({ title: t('channel.leave'), body: t('channel.leave.body', { name: channelName }), confirmLabel: t('channel.leave') }))) return
    try {
      await api.leaveChannel(channelId, userId)
      onLeft()
    } catch (failure) {
      const last = isApiError(failure) && failure.code === 'last_manager'
      setLastManager(last)
      setError(last ? t('channel.leave.lastManager') : errorText(failure, t('error.generic')))
    }
  }

  return (
    <>
      <Button type="button" variant="secondary" onClick={() => void leave()}>{t('channel.leave')}</Button>
      {error ? <p className="form-error" role="alert">{error}</p> : null}
      {lastManager && onChooseManager ? <Button type="button" onClick={onChooseManager}>{t('channel.leave.chooseManager')}</Button> : null}
      {dialog}
    </>
  )
}
