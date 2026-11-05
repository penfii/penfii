import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { clearAllNewDocDrafts } from '../docs/newDocDraft'

export function LogoutButton() {
  const { t } = useI18n()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  return (
    <button
      type="button"
      className="btn btn-secondary"
      onClick={() => {
        void api.logout().finally(() => {
          clearAllNewDocDrafts()
          queryClient.clear()
          void navigate('/login')
        })
      }}
    >
      {t('logout')}
    </button>
  )
}
