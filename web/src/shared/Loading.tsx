import { useTranslation } from 'react-i18next'

export function Loading({ className = '' }: { className?: string }) {
  const { t } = useTranslation()
  return (
    <p role="status" className={`flex items-center gap-2 text-sm text-muted-foreground ${className}`}>
      <span aria-hidden="true" className="bg-brand size-2 rounded-full motion-safe:animate-pulse" />
      {t('common.loading')}
    </p>
  )
}
