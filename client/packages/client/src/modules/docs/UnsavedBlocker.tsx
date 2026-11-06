import { useContext } from 'react'
import { UNSAFE_DataRouterContext, useBlocker } from 'react-router'
import { Button, Dialog } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'

/**
 * (Its dialog keeps clear of the history: it is the answer to a navigation the router is holding, and a Back of its own would be that navigation again.)
 * Asks before the app navigates away from a draft with unsaved changes: a link, a button that
 * navigates, the browser's back and forward, another Wiki page chosen in a tree. (A reload or closing
 * the tab is the browser's own prompt, set by the page.) Mount it only while there is something to lose.
 * It needs a data router and renders nothing without one (a plain test router).
 */
export function UnsavedBlocker({ active }: { active: boolean }) {
  return useContext(UNSAFE_DataRouterContext) ? <Blocker active={active} /> : null
}

function Blocker({ active }: { active: boolean }) {
  const { t } = useI18n()
  const blocker = useBlocker(
    ({ currentLocation, nextLocation }) =>
      active && (currentLocation.pathname !== nextLocation.pathname || currentLocation.search !== nextLocation.search),
  )
  return (
    <Dialog open={blocker.state === 'blocked'} title={t('docs.edit.discard.title')} onClose={() => blocker.reset?.()} closeWithBack={false}>
      <p>{t('docs.edit.leave')}</p>
      <div className="dialog-actions">
        <Button variant="ghost" onClick={() => blocker.reset?.()}>{t('docs.edit.keep')}</Button>
        <Button onClick={() => blocker.proceed?.()}>{t('docs.edit.leave.confirm')}</Button>
      </div>
    </Dialog>
  )
}
