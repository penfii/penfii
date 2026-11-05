import { useEffect, useState } from 'react'
import { useI18n } from '../../i18n/I18nProvider'
import { parseCsv, type CsvTable } from './csv'

type CsvState = { state: 'loading' } | { state: 'failed' } | { state: 'ready'; table: CsvTable }

/**
 * Reads the start of a CSV file and parses it. Only `bytes` are requested (a Range request), the
 * parse stops at `rows` x `columns`, and every cell is shown as text, so a big or odd file cannot
 * hurt the page.
 */
export function useCsv(src: string, limits: { bytes: number; rows: number; columns: number }): CsvState {
  const [result, setResult] = useState<{ src: string; value: CsvState }>({ src, value: { state: 'loading' } })
  useEffect(() => {
    const controller = new AbortController()
    void (async () => {
      try {
        const response = await fetch(src, {
          credentials: 'same-origin',
          headers: { Range: `bytes=0-${String(limits.bytes - 1)}`, 'X-Penfii-Client': '1' },
          signal: controller.signal,
        })
        if (!response.ok) throw new Error(String(response.status))
        const cut = response.status === 206 && Number(/\/(\d+)$/.exec(response.headers.get('Content-Range') ?? '')?.[1] ?? 0) > limits.bytes
        const text = await response.text()
        const table = parseCsv(text, { rows: limits.rows, columns: limits.columns })
        // A file cut at the byte limit may end inside a row: drop that half row.
        if (cut && table.rows.length > 1 && table.rows.length < limits.rows) table.rows.pop()
        setResult({ src, value: { state: 'ready', table: { rows: table.rows, truncated: table.truncated || cut } } })
      } catch {
        if (!controller.signal.aborted) setResult({ src, value: { state: 'failed' } })
      }
    })()
    return () => controller.abort()
  }, [src, limits.bytes, limits.rows, limits.columns])
  return result.src === src ? result.value : { state: 'loading' }
}

export function CsvTableView({ table, compact = false }: { table: CsvTable; compact?: boolean }) {
  const { t } = useI18n()
  if (table.rows.length === 0) return <p className="csv-note">{t('composer.csvEmpty')}</p>
  const [head, ...body] = table.rows
  return (
    <table className={`csv-table${compact ? ' compact' : ''}`}>
      <thead>
        <tr>{head?.map((cell, column) => <th key={column} scope="col">{cell}</th>)}</tr>
      </thead>
      <tbody>
        {body.map((row, index) => (
          <tr key={index}>{row.map((cell, column) => <td key={column}>{cell}</td>)}</tr>
        ))}
      </tbody>
    </table>
  )
}

/** The first rows of a CSV file inside a message or comment, with a way to the full table. */
export function CsvPreview({ src, onOpen }: { src: string; onOpen: () => void }) {
  const { t } = useI18n()
  const csv = useCsv(src, { bytes: 256 * 1024, rows: 6, columns: 8 })
  if (csv.state === 'failed') return null
  return (
    <div className="csv-preview">
      {csv.state === 'loading' ? <p className="csv-note">{t('loading')}</p> : <CsvTableView table={csv.table} compact />}
      <button type="button" className="link-button" onClick={onOpen}>{t('composer.csvOpen')}</button>
    </div>
  )
}
