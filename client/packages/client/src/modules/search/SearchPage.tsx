import { useEffect, useMemo, useState } from 'react'
import { Link, useNavigate, useParams, useSearchParams } from 'react-router'
import { useInfiniteQuery, useQuery } from '@tanstack/react-query'
import type { DirectoryPerson, SearchResult, SearchTab } from '@penfii/contracts'
import { isApiError } from '@penfii/api-client'
import { api } from '../../api'
import { Button, EmptyState } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { PersonRow, useStartDm } from '../../search/DirectoryResults'
import { resultHref, SEARCH_TABS } from './resultHref'
import { hasToken, replaceToken, setKey, toggleToken, valueOf } from './searchQuery'

const STATUS = ['NOT_STARTED', 'IN_PROGRESS', 'DONE', 'CANCELED'] as const
const STATUS_WORD: Record<(typeof STATUS)[number], string> = {
  NOT_STARTED: 'not-started',
  IN_PROGRESS: 'in-progress',
  DONE: 'done',
  CANCELED: 'canceled',
}

function Snippet({ result }: { result: SearchResult }) {
  if (result.snippet.length === 0) return null
  return (
    <p className="search-snippet">
      {result.snippet.map((segment, index) => (segment.hit ? <mark key={index}>{segment.text}</mark> : <span key={index}>{segment.text}</span>))}
    </p>
  )
}

/**
 * Search across Messages, Tasks, Files, Spaces, Views and People. The chips edit the same text a
 * person can type. Results are what the server would let this person open, nothing more.
 */
export function SearchPage() {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const [params, setParams] = useSearchParams()
  const navigate = useNavigate()
  const q = params.get('q') ?? ''
  const tab = (SEARCH_TABS.includes(params.get('tab') as SearchTab) ? params.get('tab') : 'all') as SearchTab
  const stateId = params.get('state') ?? ''
  const [text, setText] = useState(q)
  useEffect(() => setText(q), [q])
  const zone = useMemo(() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
    } catch {
      return 'UTC'
    }
  }, [])

  function update(next: Record<string, string | null>) {
    const following = new URLSearchParams(params)
    for (const [key, value] of Object.entries(next)) {
      if (value === null || value === '') following.delete(key)
      else following.set(key, value)
    }
    setParams(following, { replace: true })
  }

  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  const projectToken = valueOf(q, 'project')
  const project = space.data?.projects.find((item) => item.ticketPrefix?.toLowerCase() === projectToken?.toLowerCase() || item.id === projectToken)
  const workflow = useQuery({ ...queries.projectWorkflow(project?.id ?? ''), enabled: Boolean(project?.id) })

  const ready = q.trim() !== '' || stateId !== ''
  const search = useInfiniteQuery({
    queryKey: queryKeys.search(workspaceId, q, tab, stateId, zone),
    queryFn: ({ pageParam }) => api.search(workspaceId, { q, kind: tab, stateIds: stateId || undefined, cursor: pageParam, limit: 20, tz: zone }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => last.nextCursor ?? undefined,
    enabled: ready && workspaceId.length > 0,
    retry: false,
  })
  const results = search.data?.pages.flatMap((page) => page.results) ?? []
  const status = search.data?.pages[0]?.status
  const start = useStartDm(workspaceId)
  const error = search.error

  /** A Space hit says what it is: the Channel, the Project or the Team. */
  function kindLabel(result: SearchResult) {
    const spaceKind = result.kind === 'space' ? result.fields.spaceKind : null
    return spaceKind === 'PROJECT' || spaceKind === 'TEAM' || spaceKind === 'CHANNEL' ? t(`search2.kind.space.${spaceKind}`) : t(`search2.kind.${result.kind}`)
  }

  function errorMessage() {
    if (!isApiError(error)) return t('search2.error.generic')
    if (error.status === 503) return t('search2.unavailable')
    const details = error.details as { token?: string } | undefined
    const token = typeof details?.token === 'string' ? details.token : ''
    const key = `search2.error.${error.code}` as const
    const known = ['unsupported_key', 'invalid_status', 'invalid_date', 'invalid_value', 'unsupported_syntax', 'unknown_filter_value', 'query_too_long', 'invalid_cursor', 'ambiguous_filter']
    return known.includes(error.code) ? t(key as 'search2.error.invalid_value', { token }) : t('search2.error.generic')
  }
  const candidates = isApiError(error) && error.code === 'ambiguous_filter'
    ? ((error.details as { token?: string; candidates?: Array<{ id: string; name: string }> } | undefined) ?? {})
    : null

  return (
    <div className="page-scroll search-page">
      <header className="page-intro">
        <h1>{t('nav.search')}</h1>
        <p>{t('search2.prompt')}</p>
      </header>
      <form
        className="search-form"
        onSubmit={(event) => {
          event.preventDefault()
          update({ q: text })
        }}
      >
        <input
          autoFocus
          value={text}
          placeholder={t('search2.placeholder')}
          aria-label={t('search2.placeholder')}
          onChange={(event) => setText(event.target.value)}
          onBlur={() => text !== q && update({ q: text })}
        />
        <Button type="submit">{t('nav.search')}</Button>
      </form>
      <div className="chip-row" role="group" aria-label={t('search2.chips')}>
        <label>
          {t('search2.chip.status')}{' '}
          <select value={valueOf(q, 'status') ?? ''} onChange={(event) => update({ q: setKey(q, 'status', event.target.value || null) })}>
            <option value="">—</option>
            {STATUS.map((id) => <option key={id} value={STATUS_WORD[id]}>{t(`search2.status.${id}`)}</option>)}
          </select>
        </label>
        <label>
          {t('search2.chip.project')}{' '}
          <select
            value={project?.ticketPrefix ?? ''}
            onChange={(event) => update({ q: setKey(q, 'project', event.target.value || null), state: null })}
          >
            <option value="">—</option>
            {(space.data?.projects ?? []).map((item) => <option key={item.id} value={item.ticketPrefix ?? item.id}>{item.name}</option>)}
          </select>
        </label>
        {project && workflow.data ? (
          <label>
            {t('search2.state.label', { project: project.name })}{' '}
            <select value={stateId} onChange={(event) => update({ state: event.target.value || null })}>
              <option value="">{t('search2.state.any')}</option>
              {workflow.data.states.map((state) => <option key={state.id} value={state.id}>{state.name}</option>)}
            </select>
          </label>
        ) : null}
        {[
          ['assignee:me', 'search2.chip.assigneeMe'],
          ['from:me', 'search2.chip.fromMe'],
          ['has:file', 'search2.chip.hasFile'],
          ['mentions:me', 'search2.chip.mentionsMe'],
        ].map(([token, label]) => (
          <button key={token} type="button" className={hasToken(q, token) ? 'chip active' : 'chip'} aria-pressed={hasToken(q, token)} onClick={() => update({ q: toggleToken(q, token) })}>
            {t(label as 'search2.chip.assigneeMe')}
          </button>
        ))}
      </div>
      {/after:|before:/.test(q) ? <p className="hint">{t('search2.dates', { zone })}</p> : null}
      {!ready ? <EmptyState title={t('nav.search')} body={t('search2.prompt')} /> : null}
      {search.isFetching && results.length === 0 ? <p className="hint" role="status">{t('search2.loading')}</p> : null}
      {search.isError ? (
        <div className="board-error" role="alert">
          <p>{errorMessage()}</p>
          {candidates?.candidates ? (
            <ul>
              {candidates.candidates.map((candidate) => (
                <li key={candidate.id}>
                  <Button type="button" variant="secondary" onClick={() => update({ q: replaceToken(q, candidates.token ?? '', `${(candidates.token ?? '').split(':')[0]}:${candidate.id}`) })}>
                    {candidate.name}
                  </Button>
                </li>
              ))}
            </ul>
          ) : null}
          {isApiError(error) && error.status === 503 ? <Button type="button" variant="secondary" onClick={() => void search.refetch()}>{t('search2.retry')}</Button> : null}
        </div>
      ) : null}
      {status && status.unavailableKinds.length > 0 ? (
        <p className="board-note" role="status">{t('search2.indexing', { kinds: status.unavailableKinds.map((k) => t(`search2.kind.${k as 'message'}`)).join(', ') })}</p>
      ) : null}
      {status?.stale ? <p className="board-note" role="status">{t('search2.stale')}</p> : null}
      {search.isSuccess && results.length === 0 ? <EmptyState title={t('search2.empty')} body={t('search2.emptyBody')} /> : null}
      <ul className="result-list search-results">
        {results.map((result) => {
          if (result.kind === 'person') {
            const person = result.fields.person as DirectoryPerson | undefined
            if (!person) return null
            return (
              <li key={`person:${result.id}`}>
                <PersonRow person={person} selected={false} selectable={false} onToggle={() => undefined} onOpen={() => void start.open([person.id])} />
              </li>
            )
          }
          const href = resultHref(workspaceId, result)
          const body = (
            <>
              <small className="where-chip">{kindLabel(result)}</small> <strong>{result.title}</strong>
              {result.subtitle ? <small className="hint"> · {result.subtitle}</small> : null}
              <Snippet result={result} />
            </>
          )
          return (
            <li key={`${result.kind}:${result.id}`} className="result-hit">
              {href ? <Link to={href} className="saved-open" onClick={(event) => { event.preventDefault(); void navigate(href) }}>{body}</Link> : body}
            </li>
          )
        })}
      </ul>
      {search.hasNextPage ? (
        <Button type="button" variant="secondary" disabled={search.isFetchingNextPage} onClick={() => void search.fetchNextPage()}>{t('search2.more')}</Button>
      ) : null}
    </div>
  )
}
