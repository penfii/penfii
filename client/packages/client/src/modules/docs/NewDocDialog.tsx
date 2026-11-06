import { useEffect, useState } from 'react'
import { Button, Dialog } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { ShareScopePicker, type ShareScope } from './ShareScopePicker'

const WORKSPACE: ShareScope = { visibility: 'WORKSPACE', projectId: null, teamId: null }

/**
 * A new page needs a title before it exists, and says who it is shared with: where it is made (a Project's
 * tab) does not decide that. `onCreate` throws to keep the dialog open with the error.
 */
export function NewDocDialog({
  open,
  workspaceId,
  defaultTitle = '',
  suggestedProjectId = null,
  onCreate,
  onClose,
}: {
  open: boolean
  workspaceId: string
  defaultTitle?: string
  /** The Project whose tab the page is made in: offered first when the person shares it with "a Project". */
  suggestedProjectId?: string | null
  onCreate: (title: string, scope: ShareScope) => Promise<void>
  onClose: () => void
}) {
  const { t } = useI18n()
  const [title, setTitle] = useState(defaultTitle)
  const [scope, setScope] = useState<ShareScope>(WORKSPACE)
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    if (open) {
      setTitle(defaultTitle)
      setScope(WORKSPACE)
      setFailed(false)
    }
  }, [open, defaultTitle])
  const trimmed = title.trim()

  async function submit() {
    if (!trimmed || busy) return
    setBusy(true)
    setFailed(false)
    try {
      await onCreate(trimmed, scope)
    } catch {
      setFailed(true)
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} title={t('docs.new.title')} onClose={onClose} presentation="full">
      <form
        onSubmit={(event) => {
          event.preventDefault()
          void submit()
        }}
      >
        <label className="field">
          <span>{t('docs.new.name')}</span>
          <input autoFocus required value={title} aria-invalid={!trimmed} onChange={(event) => setTitle(event.target.value)} />
        </label>
        <ShareScopePicker workspaceId={workspaceId} name="new-doc-scope" value={scope} onChange={setScope} suggestedProjectId={suggestedProjectId} />
        {failed ? <p className="board-error" role="alert">{t('docs.new.error')}</p> : null}
        <div className="dialog-actions">
          <Button type="button" variant="ghost" onClick={onClose}>{t('docs.cancel')}</Button>
          <Button type="submit" disabled={!trimmed || busy}>{t('docs.new.create')}</Button>
        </div>
      </form>
    </Dialog>
  )
}
