import { useTranslation } from 'react-i18next'
import { Toaster } from '@/components/ui/sonner'

// Sonner labels its region in English by default; at the top so a toast never covers the sticky audio bar.
export function LocalizedToaster() {
  const { t } = useTranslation()
  return <Toaster position="top-center" containerAriaLabel={t('common.notifications')} />
}
