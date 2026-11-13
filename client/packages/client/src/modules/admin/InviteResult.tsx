import { useState } from 'react'
import type { InvitationCreated } from '@penfii/contracts'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'

/**
 * What came of creating or re-issuing an invitation: the link to copy, and what happened to the e-mail.
 * The link is the invitation; the e-mail is only one way of delivering it, so a failed or missing
 * e-mail never hides the link.
 */
export function InviteResult({ invitation }: { invitation: InvitationCreated }) {
  const { t } = useI18n()
  const [copied, setCopied] = useState<'ok' | 'failed' | null>(null)
  const link = `${window.location.origin}/invite/${invitation.inviteToken}`

  async function copy() {
    try {
      await navigator.clipboard.writeText(link)
      setCopied('ok')
    } catch {
      setCopied('failed')
    }
  }

  return (
    <div className="invite-result" role="status">
      <p>{t(invitation.reissued ? 'admin.invite.reissued' : 'admin.invite.created')}</p>
      <p className={invitation.emailStatus === 'FAILED' ? 'form-error' : 'hint'}>
        {invitation.emailStatus === 'SENT' ? t('admin.invite.mail.SENT', { email: invitation.email }) : null}
        {invitation.emailStatus === 'FAILED' ? t('admin.invite.mail.FAILED', { reason: invitation.emailError ?? '' }) : null}
        {invitation.emailStatus === 'NOT_CONFIGURED' ? t('admin.invite.mail.NOT_CONFIGURED') : null}
      </p>
      <div className="invite-link">
        <input readOnly aria-label={t('admin.invite.link')} value={link} onFocus={(event) => event.currentTarget.select()} />
        <Button type="button" variant="secondary" onClick={() => void copy()}>{t('admin.invite.copy')}</Button>
      </div>
      {copied === 'ok' ? <p className="hint">{t('admin.invite.copied')}</p> : null}
      {copied === 'failed' ? <p className="form-error">{t('admin.invite.copyFailed')}</p> : null}
    </div>
  )
}
