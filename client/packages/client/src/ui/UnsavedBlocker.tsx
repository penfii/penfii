import { useContext } from 'react'
import { UNSAFE_DataRouterContext, useBlocker } from 'react-router'
import { Button, Dialog } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import type { MessageKey } from '../i18n/messages'

/** The words of the question. The default is the Wiki's; another screen names its own keys. */
export type UnsavedCopy = { title: MessageKey; body: MessageKey; keep: MessageKey; leave: MessageKey; saveAndLeave: MessageKey }

const DOCS_COPY: UnsavedCopy = {
  title: 'docs.edit.discard.title',
  body: 'docs.edit.leave',
  keep: 'docs.edit.keep',
  leave: 'docs.edit.leave.confirm',
  saveAndLeave: 'docs.edit.saveAndLeave',
}

/**
 * (Its dialog keeps clear of the history: it is the answer to a navigation the router is holding, and a Back of its own would be that navigation again.)
 * Asks before the app navigates away from a draft with unsaved changes: a link, a button that
 * navigates, the browser's back and forward, another Wiki page chosen in a tree. (A reload or closing
 * the tab is the browser's own prompt, set by the page.) Mount it only while there is something to lose.
 * It needs a data router and renders nothing without one (a plain test router).
 * The safe answer ("keep editing") has the first focus; Esc is that answer.
 */
export function UnsavedBlocker({ active, onSave, copy = DOCS_COPY }: { active: boolean; /** When given, the question offers to save first; it answers whether the save worked, and only then is the navigation let through. */ onSave?: () => Promise<boolean>; copy?: UnsavedCopy }) {
  return useContext(UNSAFE_DataRouterContext) ? <Blocker active={active} onSave={onSave} copy={copy} /> : null
}

function Blocker({ active, onSave, copy }: { active: boolean; onSave?: () => Promise<boolean>; copy: UnsavedCopy }) {
  const { t } = useI18n()
  const blocker = useBlocker(
    ({ currentLocation, nextLocation }) =>
      active && (currentLocation.pathname !== nextLocation.pathname || currentLocation.search !== nextLocation.search),
  )
  return (
    <Dialog open={blocker.state === 'blocked'} title={t(copy.title)} onClose={() => blocker.reset?.()} closeWithBack={false}>
      <p>{t(copy.body)}</p>
      <div className="dialog-actions">
        <Button variant="ghost" data-autofocus onClick={() => blocker.reset?.()}>{t(copy.keep)}</Button>
        <Button variant={onSave ? 'secondary' : 'primary'} onClick={() => blocker.proceed?.()}>{t(copy.leave)}</Button>
        {onSave ? (
          <Button
            onClick={() => {
              // A failed save keeps the person here: the page shows the failure and Retry.
              void onSave().then((saved) => { if (saved) blocker.proceed?.(); else blocker.reset?.() })
            }}
          >
            {t(copy.saveAndLeave)}
          </Button>
        ) : null}
      </div>
    </Dialog>
  )
}
