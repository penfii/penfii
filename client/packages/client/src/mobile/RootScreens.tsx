import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate, useSearchParams } from 'react-router'
import { useI18n } from '../i18n/I18nProvider'
import { queries } from '../queries'
import { SEARCH_TABS } from '../modules/search/resultHref'
import { useWorkspaceRole } from '../modules/spaces/useWorkspaceRole'
import { isWorkspaceManager } from '../modules/spaces/capabilities'
import { settingsGroups } from '../shell/settingsEntries'
import { AccountPanel } from './AccountPanel'
import { PUSH_KEY } from './routes'

/** The screens a tab opens on, other than the context list the desktop sidebar already provides. */

/** Home's shortcuts: the three places that used to be a list in the context column. */
export function HomeShortcuts({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const base = `/w/${workspaceId}`
  return (
    <nav className="mobile-shortcuts" aria-label={t('mobile.homeShortcuts')}>
      <Link to={`${base}/inbox`}>{t('nav.inbox')}</Link>
      <Link to={`${base}/my-work`}>{t('shell.projects.tasks')}</Link>
      <Link to={`${base}/saved`}>{t('mobile.vault')}</Link>
    </nav>
  )
}

/** Settings root: the account block, then the same entries as the desktop settings column, each opening a full screen. */
export function SettingsRoot({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const role = useWorkspaceRole(workspaceId)
  const base = `/w/${workspaceId}/settings`
  return (
    <div className="mobile-list">
      <h2>{t('nav.settings')}</h2>
      <AccountPanel workspaceId={workspaceId} />
      {settingsGroups(isWorkspaceManager(role)).map((group) => (
        <section key={group.id}>
          <h3>{t(group.heading)}</h3>
          <ul>
            {group.entries.map((entry) => (
              // The profile is the settings root's own path, so it is marked as a pushed screen.
              <li key={entry.id}><Link to={entry.path === '' ? `${base}?${PUSH_KEY}=profile` : `${base}${entry.path}`}>{t(entry.label)}</Link></li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  )
}

/** Unsent messages, one tap from the chat list: the band opens the Drafts list of My library. */
export function ResumeBand({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const drafts = useQuery({ ...queries.drafts(workspaceId), enabled: workspaceId.length > 0 })
  const count = drafts.data?.length ?? 0
  if (count === 0) return null
  return <Link className="mobile-resume" to={`/w/${workspaceId}/drafts`}>{t('mobile.resume', { count: String(count) })}<span aria-hidden="true">›</span></Link>
}

/** Search root: the field first, then the kinds it can be narrowed to. Submitting pushes the results. */
/**
 * The search list a phone goes back to from a search: it starts with that search's text, kind and Project state, so choosing
 * another kind searches the same thing again. (`text`, not `q`: a `q` would make it the results screen.)
 */
export function searchRootFrom(workspaceId: string, search: string): string {
  const from = new URLSearchParams(search)
  const next = new URLSearchParams()
  for (const [key, as] of [['tab', 'tab'], ['q', 'text'], ['state', 'state']] as const) {
    const value = from.get(key)
    if (value) next.set(as, value)
  }
  const query = next.toString()
  return `/w/${workspaceId}/search${query ? `?${query}` : ''}`
}

export function SearchRoot({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const navigate = useNavigate()
  const [params] = useSearchParams()
  const [text, setText] = useState(params.get('text') ?? '')
  const [tab, setTab] = useState<(typeof SEARCH_TABS)[number]>(() => SEARCH_TABS.find((id) => id === params.get('tab')) ?? 'all')
  // The Project state of the search this list came back from travels with the next one.
  const state = params.get('state')
  return (
    <div className="mobile-list">
      <form role="search" onSubmit={(event) => { event.preventDefault(); if (text.trim()) void navigate(`/w/${workspaceId}/search?tab=${tab}&q=${encodeURIComponent(text.trim())}${state ? `&state=${encodeURIComponent(state)}` : ''}`) }}>
        <input type="search" className="mobile-search" enterKeyHint="search" aria-label={t('nav.search')} placeholder={t('nav.search')} value={text} onChange={(event) => setText(event.target.value)} />
      </form>
      <h2 className="mobile-group">{t('shell.search.filters')}</h2>
      <ul>
        {SEARCH_TABS.map((id) => (
          <li key={id}><button type="button" className={tab === id ? 'mobile-choice on' : 'mobile-choice'} aria-pressed={tab === id} onClick={() => setTab(id)}>{t(`search2.tab.${id}`)}</button></li>
        ))}
      </ul>
    </div>
  )
}
