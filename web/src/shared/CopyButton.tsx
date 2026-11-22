import { CopyIcon } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'

/** Copies `text` to the clipboard. `label` is the accessible name (what is copied). */
export function CopyButton({ text, label }: { text: string; label: string }) {
  const { t } = useTranslation()

  const copy = async () => {
    try {
      // `navigator.clipboard` is undefined outside a secure context (plain HTTP on the LAN).
      if (!navigator.clipboard) throw new Error('no clipboard')
      await navigator.clipboard.writeText(text)
      toast.success(t('common.copied'))
    } catch {
      toast.error(t('common.copyFailed'))
    }
  }

  return (
    <Button type="button" variant="outline" aria-label={label} onClick={() => void copy()} className="max-md:h-11">
      <CopyIcon aria-hidden="true" />
      {t('common.copy')}
    </Button>
  )
}
