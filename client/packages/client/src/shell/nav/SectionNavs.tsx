import { settingsGroups } from '../settingsEntries'
import { DocTree } from '../../modules/docs/DocTree'
import { Link,NavLink,useLocation,useSearchParams } from 'react-router'
import { inboxSearch,parseInboxLocation } from '../../modules/attention/inboxModel'
import { useUnread } from '../../modules/attention/useUnread'
import { useI18n } from '../../i18n/I18nProvider'
import { SEARCH_TABS } from '../../modules/search/resultHref'
import { isWorkspaceManager } from '../../modules/spaces/capabilities'
import { useWorkspaceRole } from '../../modules/spaces/useWorkspaceRole'

export function SettingsNav({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const role = useWorkspaceRole(workspaceId)
  return (
    <>
      {settingsGroups(isWorkspaceManager(role)).map((group) => (
        <section key={group.id} className="tree-block">
          <h3>{t(group.heading)}</h3>
          <ul>
            {group.entries.map((entry) => (
              <li key={entry.id}><NavLink end={entry.end ?? entry.path === ''} className="tree-link" to={`/w/${workspaceId}/settings${entry.path}`}>{t(entry.label)}</NavLink></li>
            ))}
          </ul>
        </section>
      ))}
    </>
  )
}

export function HomeNav({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  return (
    <ul>
      <li><NavLink className="tree-link" to={`/w/${workspaceId}/inbox`}>{t('nav.inbox')}</NavLink></li>
      <li><NavLink className="tree-link" to={`/w/${workspaceId}/my-work`}>{t('shell.projects.tasks')}</NavLink></li>
      <li><NavLink className="tree-link" to={`/w/${workspaceId}/saved`}>{t('shell.saved')}</NavLink></li>
      <li><NavLink className="tree-link" to={`/w/${workspaceId}/drafts`}>{t('shell.drafts')}</NavLink></li>
    </ul>
  )
}

export function InboxNav({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const [params] = useSearchParams()
  const { pathname } = useLocation()
  const unread = useUnread(workspaceId)
  const onInbox = pathname.replace(/\/$/, '').endsWith('/inbox')
  const current = onInbox ? parseInboxLocation(params).view : null
  const count = unread.capped ? '99+' : String(unread.count)
  const views = [
    ['inbox', 'inbox.view.inbox'],
    ['archived', 'inbox.view.archived'],
  ] as const
  return (
    <>
    <ul>
      {views.map(([id, label]) => (
        <li key={id}>
          <Link className={current === id ? 'tree-link active' : 'tree-link'} aria-current={current === id ? 'page' : undefined} to={`/w/${workspaceId}/inbox${inboxSearch({ view: id, types: [], unread: false })}`}>
            {t(label)}
            {id === 'inbox' && unread.count > 0 ? (
              <b className="nav-count">
                <span aria-hidden="true">{count}</span>
                <span className="visually-hidden">{t('inbox.unreadBadge', { count })}</span>
              </b>
            ) : null}
          </Link>
        </li>
      ))}
    </ul>
    <section className="tree-block">
      <h3>{t('shell.mySpace')}</h3>
      <ul>
        <li><NavLink className="tree-link" to={`/w/${workspaceId}/my-work`}>{t('shell.projects.tasks')}</NavLink></li>
        <li><NavLink className="tree-link" to={`/w/${workspaceId}/saved`}>{t('shell.saved')}</NavLink></li>
        <li><NavLink className="tree-link" to={`/w/${workspaceId}/drafts`}>{t('shell.drafts')}</NavLink></li>
      </ul>
    </section>
    </>
  )
}


export function DocsNav({ workspaceId }: { workspaceId: string }) {
  const [params, setParams] = useSearchParams()
  const selected = params.get('doc')
  return (
    <DocTree
      workspaceId={workspaceId}
      selectedId={selected === 'new' ? null : selected}
      onSelect={(id, options) => {
        const next = new URLSearchParams(params)
        next.delete('draft')
        next.delete('parent')
        if (id) next.set('doc', id)
        else next.delete('doc')
        if (id && options?.edit) next.set('edit', '1')
        else next.delete('edit')
        setParams(next)
      }}
      onDraft={(parentId) => {
        const next = new URLSearchParams(params)
        next.delete('doc')
        next.delete('edit')
        next.set('draft', '1')
        if (parentId) next.set('parent', parentId)
        else next.delete('parent')
        setParams(next)
      }}
    />
  )
}

export function SearchNav({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const [params] = useSearchParams()
  const tab = params.get('tab') ?? 'all'
  const q = params.get('q') ?? ''
  return (
    <section className="tree-block">
      <h3>{t('shell.search.filters')}</h3>
      <ul>
        {SEARCH_TABS.map((id) => (
          <li key={id}>
            {/* Every tab shares one path, so a NavLink would mark all of them current; the tab parameter decides. */}
            <Link className={tab === id ? 'tree-link active' : 'tree-link'} aria-current={tab === id ? 'page' : undefined} to={`/w/${workspaceId}/search?tab=${id}${q ? `&q=${encodeURIComponent(q)}` : ''}`}>{t(`search2.tab.${id}`)}</Link>
          </li>
        ))}
      </ul>
    </section>
  )
}
