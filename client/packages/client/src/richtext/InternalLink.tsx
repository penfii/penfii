import type { MouseEvent, ReactNode } from 'react'
import { useI18n } from '../i18n/I18nProvider'
import { useResolverVersion, useRichTextEnv, type RichTextEnv } from './env'
import { isBareUrlText, parseInternalLink, type InternalTarget } from './internalLink'

type Shown = { kind: 'title'; title: string } | { kind: 'loading' } | { kind: 'unavailable' }

const MARK: Record<InternalTarget['kind'], string> = { channel: '#', thread: '↳', project: '▣', doc: '▤', ticket: '◆' }

function resolveTitle(target: InternalTarget, env: RichTextEnv, threadIn: (channel: string) => string, threadPlain: string): Shown {
  // A link into another workspace is never resolved here: nothing about it is read or shown.
  if (target.workspaceId !== env.workspaceId) return { kind: 'unavailable' }
  if (target.kind === 'doc' || target.kind === 'ticket') {
    const answer = env.resolver.want(target.kind === 'doc' ? 'DOC' : 'TICKET', target.kind === 'doc' ? target.docId : target.ticketId)
    if (!answer) return { kind: 'loading' }
    return answer.available && answer.title ? { kind: 'title', title: answer.title } : { kind: 'unavailable' }
  }
  const space = env.space
  if (!space) return { kind: 'loading' }
  if (target.kind === 'project') {
    const project = space.projects.find((item) => item.id === target.projectId)
    return project ? { kind: 'title', title: project.name } : { kind: 'unavailable' }
  }
  const channel = space.channels.find((item) => item.conversationId === target.conversationId)
  const dm = space.dms.find((item) => item.conversationId === target.conversationId)
  const name = channel ? channel.name : dm ? dm.peers.map((peer) => peer.displayName).join(', ') : null
  if (name === null) return { kind: 'unavailable' }
  return { kind: 'title', title: target.kind === 'thread' ? (channel ? threadIn(`# ${name}`) : threadPlain) : name }
}

function onPlainClick(event: MouseEvent<HTMLAnchorElement>, to: string, navigate: (to: string) => void) {
  // The browser keeps new-tab behaviour: modified or non-primary clicks are not ours.
  if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return
  event.preventDefault()
  navigate(to)
}

/**
 * A link drawn inside the rich text. An in-app URL with nothing written beside it becomes a chip with
 * the target's name; one with the person's own words keeps those words. Either way the href is the
 * saved URL, a plain click moves within the app and a new-tab click is left to the browser.
 * Anything else (another site, another origin) is an ordinary new-tab link.
 */
export function LinkView({ href, text, children }: { href: string; text: string; children: ReactNode }) {
  const { t } = useI18n()
  const env = useRichTextEnv()
  useResolverVersion(env)
  const target = env ? parseInternalLink(href, window.location.origin) : null
  if (!env || !target) {
    return <a href={href} target="_blank" rel="noreferrer">{children}</a>
  }
  const url = new URL(href)
  const to = url.pathname + url.search + url.hash
  if (!isBareUrlText(text, href)) {
    return <a href={href} onClick={(event) => onPlainClick(event, to, env.navigate)}>{children}</a>
  }
  const shown = resolveTitle(target, env, (channel) => t('link.threadIn', { channel }), t('link.thread'))
  const label = shown.kind === 'title' ? shown.title : shown.kind === 'loading' ? '…' : t('link.unavailable')
  return (
    <a
      href={href}
      title={shown.kind === 'title' ? shown.title : href}
      className={['link-chip', `link-chip-${target.kind}`, shown.kind === 'unavailable' ? 'is-unavailable' : ''].filter(Boolean).join(' ')}
      data-link-kind={target.kind}
      onClick={(event) => onPlainClick(event, to, env.navigate)}
    >
      <span aria-hidden="true" className="link-chip-mark">{MARK[target.kind]}</span> {label}
    </a>
  )
}
