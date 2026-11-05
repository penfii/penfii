import { useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import { Dialog } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { resultHref } from '../search/resultHref'
import { queries } from '../../queries'

type Entry = { id: string; label: string; hint?: string; go: () => void }

/**
 * Quick jump: Ctrl/⌘+K opens one box that goes to the person's own places or to a search result.
 * It runs nothing and changes nothing; it only navigates.
 */
export function CommandPalette({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const navigate = useNavigate()
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)
  const input = useRef<HTMLInputElement>(null)

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        setOpen((value) => !value)
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [])
  useEffect(() => {
    if (open) {
      setQuery('')
      setActive(0)
      window.setTimeout(() => input.current?.focus(), 0)
    }
  }, [open])

  const trimmed = query.trim()
  const [debounced, setDebounced] = useState('')
  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(trimmed), 250)
    return () => window.clearTimeout(timer)
  }, [trimmed])
  const remote = useQuery({ ...queries.palette(workspaceId, debounced), enabled: open && debounced.length > 0 })

  const entries = useMemo<Entry[]>(() => {
    const base = `/w/${workspaceId}`
    const close = () => setOpen(false)
    const places: Entry[] = [
      { id: 'inbox', label: t('palette.inbox'), go: () => navigate(`${base}/inbox`) },
      { id: 'mywork', label: t('palette.myWork'), go: () => navigate(`${base}/my-work`) },
      { id: 'saved', label: t('palette.saved'), go: () => navigate(`${base}/saved`) },
      { id: 'drafts', label: t('palette.drafts'), go: () => navigate(`${base}/drafts`) },
    ].filter((item) => trimmed === '' || item.label.toLowerCase().includes(trimmed.toLowerCase()))
    const found: Entry[] = (remote.data?.results ?? []).flatMap((result) => {
      const href = resultHref(workspaceId, result)
      return href ? [{ id: `${result.kind}:${result.id}`, label: result.title, hint: result.subtitle ?? t(`search2.kind.${result.kind}`), go: () => navigate(href) }] : []
    })
    const all: Entry[] = [...places, ...found]
    if (trimmed !== '') all.push({ id: 'search', label: t('palette.search', { query: trimmed }), go: () => void navigate(`${base}/search?q=${encodeURIComponent(trimmed)}`) })
    return all.map((entry) => ({ ...entry, go: () => { close(); entry.go() } }))
  }, [workspaceId, trimmed, remote.data, navigate, t])

  return (
    <Dialog open={open} title={t('palette.title')} onClose={() => setOpen(false)}>
      <div className="palette">
        <input
          ref={input}
          value={query}
          role="combobox"
          aria-expanded="true"
          aria-controls="palette-list"
          aria-activedescendant={entries[active] ? `palette-${entries[active].id}` : undefined}
          placeholder={t('palette.placeholder')}
          aria-label={t('palette.placeholder')}
          onChange={(event) => { setQuery(event.target.value); setActive(0) }}
          onKeyDown={(event) => {
            if (event.key === 'ArrowDown') { event.preventDefault(); setActive((value) => Math.min(entries.length - 1, value + 1)) }
            if (event.key === 'ArrowUp') { event.preventDefault(); setActive((value) => Math.max(0, value - 1)) }
            if (event.key === 'Enter') { event.preventDefault(); entries[active]?.go() }
          }}
        />
        <ul className="palette-list" id="palette-list" role="listbox">
          {entries.map((entry, index) => (
            <li key={entry.id} id={`palette-${entry.id}`} role="option" aria-selected={index === active}>
              <button type="button" tabIndex={-1} aria-selected={index === active} onMouseEnter={() => setActive(index)} onClick={entry.go}>
                <strong>{entry.label}</strong>
                {entry.hint ? <small className="hint"> · {entry.hint}</small> : null}
              </button>
            </li>
          ))}
          {entries.length === 0 ? <li className="hint">{t('palette.empty')}</li> : null}
        </ul>
        <small className="hint">{t('palette.hint')}</small>
      </div>
    </Dialog>
  )
}
