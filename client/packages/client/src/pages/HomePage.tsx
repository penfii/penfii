import { useQuery } from '@tanstack/react-query'
import { Link, useParams } from 'react-router'
import type { ScopedTicket } from '@penfii/contracts'
import { formatMessageTime } from '../format'
import { useI18n } from '../i18n/I18nProvider'
import { useSession } from '../modules/identity'
import { queries } from '../queries'
import { recentChats, recentDocs, recentProjects, todayList, toDoList, localDay, type RecentItem } from './homeModel'

const SHOWN = 6

/**
 * Where a person starts: what needs doing now, today's schedule, and the work touched lately. Everything is read
 * from endpoints the other screens already use; each row opens the screen it came from.
 */
export function HomePage() {
  const { t, locale } = useI18n()
  const { workspaceId = '' } = useParams()
  const session = useSession()
  const workspaceName = useQuery(queries.workspaces()).data?.find((item) => item.id === workspaceId)?.name
  const enabled = workspaceId.length > 0
  const space = useQuery({ ...queries.space(workspaceId), enabled })
  const mine = useQuery({ ...queries.homeWork(workspaceId), enabled })
  const docs = useQuery({ ...queries.docTree(workspaceId), enabled })
  const tickets = mine.data?.tickets ?? []
  const todo = toDoList(tickets)
  const today = todayList(tickets, localDay(new Date()))
  const recent = {
    docs: recentDocs(docs.data ?? [], workspaceId, 5),
    chats: recentChats(space.data?.dms ?? [], space.data?.channels ?? [], workspaceId, 5),
    projects: recentProjects(tickets, space.data?.projects ?? [], workspaceId, 5),
  }

  return (
    <div className="page-scroll home-page">
      <header className="page-intro">
        <h1>{workspaceName ?? t('nav.home')}</h1>
        <p className="hint">{t('home.hello', { name: session.data?.displayName ?? '' })}</p>
      </header>
      <div className="home-columns">
        <HomeSection title={t('home.todo')} count={todo.length} more={{ to: `/w/${workspaceId}/my-work`, label: t('home.todoAll') }}>
          <TicketRows workspaceId={workspaceId} today={localDay(new Date())} tickets={todo.slice(0, SHOWN)} pending={mine.isPending} failed={mine.isError} empty={t('home.todoEmpty')} />
        </HomeSection>
        <HomeSection title={t('home.today')} count={today.length}>
          <TicketRows workspaceId={workspaceId} today={localDay(new Date())} tickets={today.slice(0, SHOWN)} pending={mine.isPending} failed={mine.isError} empty={t('home.todayEmpty')} />
        </HomeSection>
      </div>
      <section className="home-section" aria-label={t('home.recent')}>
        <h2>{t('home.recent')}</h2>
        <div className="home-columns three">
          <RecentColumn title={t('nav.docs')} items={recent.docs} empty={t('home.recentEmpty')} pending={docs.isPending} locale={locale} fallbackTitle={t('docs.untitled')} />
          <RecentColumn title={t('nav.chat')} items={recent.chats} empty={t('home.recentEmpty')} pending={space.isPending} locale={locale} fallbackTitle="" />
          <RecentColumn title={t('nav.projects')} items={recent.projects} empty={t('home.recentEmpty')} pending={mine.isPending || space.isPending} locale={locale} fallbackTitle="" />
        </div>
      </section>
    </div>
  )
}

function HomeSection({ title, count, more, children }: { title: string; count?: number; more?: { to: string; label: string }; children: React.ReactNode }) {
  return (
    <section className="home-section" aria-label={title}>
      <div className="home-section-head">
        <h2>{title}{count ? <span className="home-count">{count}</span> : null}</h2>
        {more ? <Link to={more.to}>{more.label}</Link> : null}
      </div>
      {children}
    </section>
  )
}

function TicketRows({ workspaceId, today, tickets, pending, failed, empty }: { workspaceId: string; today: string; tickets: ScopedTicket[]; pending: boolean; failed: boolean; empty: string }) {
  const { t } = useI18n()
  if (pending) return <p className="hint" role="status">{t('loading')}</p>
  if (failed) return <p className="hint" role="alert">{t('error.generic')}</p>
  if (tickets.length === 0) return <p className="hint">{empty}</p>
  return (
    <ul className="home-list tickets">
      {tickets.map((ticket) => (
        <li key={ticket.id}>
          <Link className={ticket.dueOn !== null && ticket.dueOn <= today ? 'is-due' : undefined} to={`/w/${workspaceId}/projects/${ticket.projectId}/work?panel=ticket:${ticket.id}`}>
            <span className="home-row-title"><b>{ticket.key}</b> {ticket.title}</span>
            <small className="hint">{ticket.projectName} · {ticket.stateName}</small>
            {ticket.dueOn ? <span className="home-due">{ticket.dueOn === today ? t('home.dueToday') : ticket.dueOn}</span> : null}
          </Link>
        </li>
      ))}
    </ul>
  )
}

function RecentColumn({ title, items, empty, pending, locale, fallbackTitle }: { title: string; items: RecentItem[]; empty: string; pending: boolean; locale: string; fallbackTitle: string }) {
  return (
    <div className="home-recent">
      <h3>{title}</h3>
      {pending ? null : items.length === 0 ? <p className="hint">{empty}</p> : null}
      <ul className="home-list">
        {items.map((item) => (
          <li key={item.id}>
            <Link to={item.to}>
              <span className="home-row-title">{item.title || fallbackTitle}</span>
              <small className="hint">{item.hint ? `${item.hint} · ` : ''}{item.at ? formatMessageTime(item.at, locale) : ''}</small>
            </Link>
          </li>
        ))}
      </ul>
    </div>
  )
}
