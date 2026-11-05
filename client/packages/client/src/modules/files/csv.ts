/** A small CSV reader for previews: quoted fields, doubled quotes, CRLF, a BOM, `,` `;` or tab. Cells stay text. */
export type CsvTable = { rows: string[][]; truncated: boolean }

function delimiterOf(text: string): string {
  const line = text.slice(0, text.search(/\r|\n|$/))
  let best = ','
  let most = 0
  for (const candidate of [',', ';', '\t']) {
    const count = line.split(candidate).length - 1
    if (count > most) {
      best = candidate
      most = count
    }
  }
  return best
}

export function parseCsv(input: string, limits: { rows: number; columns: number }): CsvTable {
  const text = input.charCodeAt(0) === 0xfeff ? input.slice(1) : input
  const delimiter = delimiterOf(text)
  const rows: string[][] = []
  let row: string[] = []
  let cell = ''
  let quoted = false
  let truncated = false
  const endCell = () => {
    if (row.length < limits.columns) row.push(cell)
    else truncated = true
    cell = ''
  }
  const endRow = () => {
    endCell()
    rows.push(row)
    row = []
  }
  for (let index = 0; index < text.length; index += 1) {
    if (rows.length >= limits.rows) {
      truncated = true
      break
    }
    const char = text[index]
    if (quoted) {
      if (char === '"') {
        if (text[index + 1] === '"') {
          cell += '"'
          index += 1
        } else quoted = false
      } else cell += char
    } else if (char === '"' && cell === '') quoted = true
    else if (char === delimiter) endCell()
    else if (char === '\n' || char === '\r') {
      if (char === '\r' && text[index + 1] === '\n') index += 1
      endRow()
    } else cell += char
  }
  if (rows.length < limits.rows && (cell !== '' || row.length > 0)) endRow()
  return { rows: rows.filter((r) => r.length > 1 || r[0] !== ''), truncated }
}
