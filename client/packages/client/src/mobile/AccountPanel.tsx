import { useState } from 'react'
import { useNavigate } from 'react-router'
import { Avatar } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import { LogoutButton, useSession } from '../modules/identity'
import { HelpDialog, StatusDialog, WorkspaceCreate, useActiveWorkspace, useSnooze } from '../shell/account'
import { Sheet, SheetRow } from '../design/Sheet'

/**
 * The account menu of the desktop rail, as rows at the top of Settings: who is signed in, status,
 * Do Not Disturb, the workspace switcher, help and logout. The actions are the desktop's own
 * (`shell/account`); only the presentation differs.
 */
export function AccountPanel({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const navigate = useNavigate()
  const session = useSession()
  const { workspaces, workspace } = useActiveWorkspace(workspaceId)
  const snooze = useSnooze()
  const [statusOpen, setStatusOpen] = useState(false)
  const [helpOpen, setHelpOpen] = useState(false)
  const [switching, setSwitching] = useState(false)
  const [quiet, setQuiet] = useState(false)
  const user = session.data
  const name = user?.displayName ?? ''
  const status = [user?.statusEmoji, user?.statusText].filter(Boolean).join(' ')
  return (
    <section className="mobile-account" aria-label={t('account.title')}>
      <div className="mobile-account-id">
        <Avatar name={name || t('account.title')} userId={user?.id} hasAvatar={user?.hasAvatar} />
        <span>
          <strong>{name}</strong>
          {user?.email ? <small>{user.email}</small> : null}
          {status ? <small>{status}</small> : null}
        </span>
      </div>
      <ul className="mobile-account-rows">
        <li><button type="button" onClick={() => setStatusOpen(true)}>{t('menu.status')}</button></li>
        <li>
          <button type="button" aria-pressed={quiet} onClick={() => void snooze().then((saved) => saved && setQuiet(true))}>
            {t('menu.dnd')}{quiet ? <span aria-hidden="true"> ✓</span> : null}
          </button>
        </li>
        <li><button type="button" aria-haspopup="dialog" onClick={() => setSwitching(true)}>{t('menu.switch')}{workspace ? <small>{workspace.name}</small> : null}</button></li>
        <li><button type="button" onClick={() => setHelpOpen(true)}>{t('help.open')}</button></li>
        <li className="mobile-account-logout"><LogoutButton /></li>
      </ul>
      <Sheet open={switching} title={t('menu.switch')} onClose={() => setSwitching(false)}>
        {(workspaces.data ?? []).map((item) => (
          <SheetRow key={item.id} selected={item.id === workspaceId} onClick={() => { setSwitching(false); void navigate(`/w/${item.id}`) }}>{item.name}</SheetRow>
        ))}
        <WorkspaceCreate buttonClassName="m-sheet-row" />
      </Sheet>
      <StatusDialog
        open={statusOpen}
        onClose={() => setStatusOpen(false)}
        displayName={name}
        jobTitle={user?.jobTitle ?? ''}
        timeZone={user?.timeZone ?? ''}
        statusEmoji={user?.statusEmoji ?? ''}
        statusText={user?.statusText ?? ''}
        statusClearAt={user?.statusClearAt ?? ''}
      />
      <HelpDialog open={helpOpen} onClose={() => setHelpOpen(false)} />
    </section>
  )
}
