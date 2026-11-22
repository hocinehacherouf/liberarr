import { useTranslation } from 'react-i18next'
import { Spinner } from '@/components/ui/spinner'

export function Loading({ className = '' }: { className?: string }) {
  const { t } = useTranslation()
  return (
    <div className={className}>
      <Spinner aria-label={t('common.loading')} className="text-muted-foreground" />
    </div>
  )
}
