import { useEffect, useState } from 'react'
import type { PrefixCheck } from '@penfii/contracts'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'

const DEBOUNCE_MS = 300

/**
 * The Ticket Prefix field of a new Project: it follows a suggestion made from the name until the
 * person types their own, and it only reports "usable" after the server says the value is free.
 */
export function useTicketPrefix(workspaceId: string, name: string) {
  const { t } = useI18n()
  const [value, setValue] = useState('')
  const [touched, setTouched] = useState(false)
  const [check, setCheck] = useState<PrefixCheck | null>(null)
  const trimmedName = name.trim()

  useEffect(() => {
    if (!trimmedName && !touched) return
    let cancelled = false
    const timer = window.setTimeout(() => {
      api
        .checkProjectPrefix(workspaceId, { name: trimmedName || undefined, prefix: touched ? value : undefined })
        .then((result) => {
          if (cancelled) return
          setCheck(result)
          if (!touched) setValue(result.suggestion ?? '')
        })
        .catch(() => {
          if (!cancelled) setCheck(null)
        })
    }, DEBOUNCE_MS)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [workspaceId, trimmedName, touched, value])

  const canonical = value.trim().toUpperCase()
  const status = check?.prefix
  const usable = touched
    ? status?.value === canonical && status.valid && status.available
    : canonical.length > 0 && check?.suggestion === canonical

  let hint = t('work.prefix.help')
  if (touched && status && status.value === canonical) {
    hint = !status.valid ? t('work.prefix.invalid') : !status.available ? t('work.prefix.taken') : t('work.prefix.ok')
  } else if (!touched && trimmedName && check && check.suggestion === null) {
    hint = t('work.prefix.manual')
  }

  return {
    value,
    touched,
    usable: Boolean(usable),
    hint,
    edit(next: string) {
      setTouched(true)
      setValue(next.toUpperCase())
    },
  }
}
