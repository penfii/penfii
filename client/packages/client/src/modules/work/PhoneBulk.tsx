import { useState } from 'react'
import type { BulkResult } from '@penfii/contracts'
import { Sheet, SheetRow } from '../../design/Sheet'
import { useI18n } from '../../i18n/I18nProvider'
import { BULK_FIELDS, BulkResultView, useBulkForm, type Field } from './BulkBar'
import type { WorkTicket } from './viewModel'

/**
 * Select mode on a phone: the List's rows get a check, and this bar holds what can be done with
 * them. The choice of a change is a sheet (field, then value) and goes through the same request as
 * the Table's bulk bar. Nothing here deletes: the server has no bulk delete.
 */
export function PhoneBulk({
  workspaceId,
  selected,
  result,
  names,
  onAll,
  onDone,
  onResult,
}: {
  workspaceId: string
  selected: WorkTicket[]
  result: BulkResult | null
  names: Map<string, { key: string; title: string }>
  /** Chooses every Ticket that may be changed, or none when they are all chosen already. */
  onAll: () => void
  onDone: () => void
  onResult: (result: BulkResult | null) => void
}) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const [step, setStep] = useState<'field' | 'value'>('field')
  // "No priority" is a real value, so a change is applied only after a value was tapped, never by default.
  const [picked, setPicked] = useState(false)
  const form = useBulkForm(workspaceId, selected, (outcome) => {
    onResult(outcome)
    if (outcome) setOpen(false)
  })
  const { field, setField, value, setValue, options, locked, ready, apply, bulk } = form

  function close() {
    setOpen(false)
    setStep('field')
  }

  return (
    <>
      <BulkResultView result={result} names={names} />
      <div className="phone-bulk" role="region" aria-label={t('bulk.title')}>
        <strong>{selected.length > 0 ? t('bulk.selected', { count: String(selected.length) }) : t('bulk.pickTickets')}</strong>
        <button type="button" className="btn btn-ghost" onClick={onAll}>{t('bulk.all')}</button>
        <button type="button" className="btn btn-primary" disabled={selected.length === 0} aria-haspopup="dialog" onClick={() => { setStep('field'); setOpen(true) }}>{t('bulk.change')}</button>
        <button type="button" className="btn btn-ghost" onClick={onDone}>{t('bulk.done')}</button>
      </div>
      <Sheet open={open} title={step === 'field' ? t('bulk.title') : t(`bulk.field.${field}`)} onClose={close}>
        {step === 'field' ? (
          BULK_FIELDS.map((item: Field) => (
            <SheetRow key={item} selected={item === field} onClick={() => { setField(item); setValue(''); setPicked(false); setStep('value') }}>{t(`bulk.field.${item}`)}</SheetRow>
          ))
        ) : (
          <>
            <SheetRow onClick={() => setStep('field')}>{t('bulk.field')}</SheetRow>
            {locked ? <p className="field-hint">{t('bulk.oneProject')}</p> : null}
            {options.length === 0 && !locked ? <p className="field-hint">{t('bulk.noOptions')}</p> : null}
            {options.map((option) => (
              <SheetRow key={option.value} selected={picked && option.value === value} onClick={() => { setValue(option.value); setPicked(true) }}>{option.label}</SheetRow>
            ))}
            {bulk.isError ? <p className="board-error" role="alert">{t('bulk.error')}</p> : null}
            <button type="button" className="btn btn-primary m-sheet-apply" disabled={!ready || !picked || bulk.isPending} onClick={apply}>{t('bulk.applyTo', { count: String(selected.length) })}</button>
          </>
        )}
      </Sheet>
    </>
  )
}
