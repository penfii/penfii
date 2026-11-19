import { useI18n } from '../../i18n/I18nProvider'

/** A check that shows "some" as a dash and says so in words for assistive technology (colour is never the only sign). */
export function TriCheck({ label, ids, selection, onSelect }: { label: string; ids: string[]; selection: ReadonlySet<string>; onSelect: (ids: string[], on: boolean) => void }) {
  const { t } = useI18n()
  const count = ids.filter((id) => selection.has(id)).length
  const some = count > 0 && count < ids.length
  return (
    <input
      type="checkbox"
      aria-label={some ? `${label} (${t('work.table.partial', { picked: String(count), total: String(ids.length) })})` : label}
      aria-checked={some ? 'mixed' : count === ids.length && ids.length > 0}
      checked={ids.length > 0 && count === ids.length}
      ref={(node) => {
        if (node) node.indeterminate = some
      }}
      onChange={() => onSelect(ids, count < ids.length)}
    />
  )
}
