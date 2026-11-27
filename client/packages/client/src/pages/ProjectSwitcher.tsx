import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { Link, useNavigate } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import type { FavoriteItem, Project } from '@penfii/contracts'
import { useIsMobile } from '../design/useIsMobile'
import { useI18n } from '../i18n/I18nProvider'
import { useSession } from '../modules/identity'
import { Popover } from '../modules/work/Popover'
import { queries } from '../queries'
import { readRecentProjects, recordRecentProject, switchTarget, switcherSections, takesPart } from './projectsModel'

/**
 * Remembers, on this device, which Projects the person really opened: only Projects they take part in, newest first. This
 * is a separate thing from a favorite (which the person chose) and from taking part (which the server says).
 */
export function useRecordProjectVisit(workspaceId: string, projectId: string, enabled: boolean) {
  const session = useSession()
  const userId = session.data?.id
  useEffect(() => {
    if (enabled && userId) recordRecentProject(workspaceId, userId, projectId)
  }, [enabled, workspaceId, projectId, userId])
}

/**
 * The Project's name as a switcher, when the person takes part in more than one Project. With only this one there is
 * nothing to switch to, so the name stays plain text.
 */
export function ProjectTitle({ workspaceId, projectId, name, section, projects }: { workspaceId: string; projectId: string; name: string; section: string; projects: Project[] }) {
  const participating = projects.filter(takesPart)
  if (participating.length < 2) return <TitleName name={name} plain><span className="pop-label" data-fit>{name}</span></TitleName>
  return <TitleName name={name}><ProjectSwitcher workspaceId={workspaceId} projectId={projectId} name={name} section={section} projects={projects} /></TitleName>
}

/**
 * The Project's name on one line: it shows whole while there is room and ends in an ellipsis only when there is none (the
 * header never grows taller, and the actions beside it keep their place). The whole name is then in a tooltip that opens on
 * hover and on keyboard focus (the name takes focus when it is cut and is not a button already).
 */
function TitleName({ name, plain = false, children }: { name: string; plain?: boolean; children: ReactNode }) {
  const box = useRef<HTMLSpanElement>(null)
  const [cut, setCut] = useState(false)
  useLayoutEffect(() => {
    const root = box.current
    if (!root) return
    const measure = () => {
      const fit = root.querySelector<HTMLElement>('[data-fit]')
      setCut(fit !== null && fit.scrollWidth > fit.clientWidth + 1)
    }
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(root)
    return () => observer.disconnect()
  }, [name])
  return (
    <span ref={box} className="project-title-name" data-full={cut ? name : undefined} tabIndex={cut && plain ? 0 : undefined}>
      {children}
    </span>
  )
}

function ProjectSwitcher({ workspaceId, projectId, name, section, projects }: { workspaceId: string; projectId: string; name: string; section: string; projects: Project[] }) {
  const favorites = useQuery({ ...queries.favorites(workspaceId), enabled: workspaceId.length > 0 })
  return (
    <Popover label={name} className="project-switch-trigger" wide truncate badge={<span aria-hidden="true" className="project-switch-caret"> ⌄</span>}>
      {(close) => (
        <SwitcherPanel workspaceId={workspaceId} projectId={projectId} section={section} projects={projects} favorites={favorites.data ?? []} onClose={close} />
      )}
    </Popover>
  )
}

function SwitcherPanel({
  workspaceId,
  projectId,
  section,
  projects,
  favorites,
  onClose,
}: {
  workspaceId: string
  projectId: string
  section: string
  projects: Project[]
  favorites: FavoriteItem[]
  onClose: () => void
}) {
  const { t } = useI18n()
  const session = useSession()
  const mobile = useIsMobile()
  const navigate = useNavigate()
  const [query, setQuery] = useState('')
  const list = useRef<HTMLDivElement>(null)
  const search = useRef<HTMLInputElement>(null)
  // The keyboard of a phone would cover the list, so the search is not focused there until it is tapped.
  useEffect(() => {
    if (!mobile) search.current?.focus()
  }, [mobile])
  const recentIds = session.data?.id ? readRecentProjects(workspaceId, session.data.id) : []
  const sections = switcherSections(projects, favorites, recentIds, query)
  const total = sections.favorites.length + sections.recent.length + sections.others.length
  const groups: Array<[string, Project[]]> = [
    [t('projects.favorites'), sections.favorites],
    [t('projects.switch.recent'), sections.recent],
    [t('projects.switch.others'), sections.others],
  ]
  const links = () => Array.from(list.current?.querySelectorAll<HTMLElement>('a[data-switch]') ?? [])
  function onKeyDown(event: KeyboardEvent) {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    const all = links()
    const index = all.findIndex((node) => node === document.activeElement)
    const next = event.key === 'ArrowDown' ? index + 1 : index - 1
    event.preventDefault()
    if (next < 0) search.current?.focus()
    else all[Math.min(next, all.length - 1)]?.focus()
  }
  function pick(target: Project) {
    onClose()
    if (target.id !== projectId) void navigate(switchTarget(workspaceId, target, section))
  }
  return (
    <div className="project-switch" ref={list} onKeyDown={onKeyDown}>
      <input
        ref={search}
        type="search"
        className="view-search"
        aria-label={t('projects.search')}
        placeholder={t('projects.search')}
        value={query}
        onChange={(event) => setQuery(event.target.value)}
      />
      {total === 0 ? (
        <p className="hint" role="status">{t('projects.switch.none')}</p>
      ) : (
        groups.map(([title, items]) =>
          items.length === 0 ? null : (
            <section key={title} aria-label={title}>
              <h3 className="project-switch-title">{title}</h3>
              <ul className="project-switch-list">
                {items.map((project) => (
                  <li key={project.id}>
                    <Link
                      data-switch
                      to={switchTarget(workspaceId, project, section)}
                      aria-current={project.id === projectId ? 'true' : undefined}
                      className={project.id === projectId ? 'project-switch-item current' : 'project-switch-item'}
                      onClick={(event) => {
                        event.preventDefault()
                        pick(project)
                      }}
                    >
                      <span>{project.name}</span>
                      {project.id === projectId ? <small>{t('projects.switch.current')}</small> : project.ticketPrefix ? <small>{project.ticketPrefix}</small> : null}
                    </Link>
                  </li>
                ))}
              </ul>
            </section>
          ),
        )
      )}
      <Link className="project-switch-all" to={`/w/${workspaceId}/projects?browse=all${query.trim() && total === 0 ? `&q=${encodeURIComponent(query.trim())}` : ''}`} onClick={onClose}>
        {t('projects.switch.all')}
      </Link>
    </div>
  )
}
