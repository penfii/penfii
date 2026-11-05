import { useNavigate, useSearchParams } from 'react-router'
import type { InvitationTeamOutcome } from '@penfii/contracts'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'

/** The card shown once an invitation is accepted: the Workspace was joined, and what became of the Team step. */
export function JoinedCard({ outcome, onContinue }: { outcome: InvitationTeamOutcome | null; onContinue: () => void }) {
  const { t } = useI18n()
  return (
    <div className="card">
      <h1>{t('invite.title')}</h1>
      <p>{t('invite.done')}</p>
      {outcome ? (
        <p role="status" className="invite-team-outcome">
          {outcome.status === 'JOINED' ? t('invite.result.joined', { team: outcome.teamName }) : t('invite.result.skipped')}
        </p>
      ) : null}
      <Button type="button" onClick={onContinue}>{t('invite.continue')}</Button>
    </div>
  )
}

/** Where a provider sign-in lands after accepting an invitation; the Team outcome arrives in the address. */
export function InviteJoined() {
  const navigate = useNavigate()
  const [params] = useSearchParams()
  const status = params.get('teamOutcome')
  const outcome: InvitationTeamOutcome | null =
    status === 'JOINED' || status === 'ARCHIVED' || status === 'GONE'
      ? { teamId: null, teamName: params.get('teamName') ?? '', status }
      : null
  return <JoinedCard outcome={outcome} onContinue={() => void navigate('/')} />
}
