import { EllipsisIcon } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'

const RERUN_STEPS = ['transcribe', 'summarize', 'diarize', 'export'] as const

/** The "more" menu of a recording: run a step again. */
export function RecordingMenu({ disabled, onRerun }: { disabled: boolean; onRerun: (step: string) => void }) {
  const { t } = useTranslation()
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            disabled={disabled}
            aria-label={t('recordings.rerunTitle')}
            className="shrink-0 max-md:size-11"
          />
        }
      >
        <EllipsisIcon />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-auto">
        <DropdownMenuGroup>
          <DropdownMenuLabel>{t('recordings.rerunHelp')}</DropdownMenuLabel>
          {RERUN_STEPS.map((step) => (
            <DropdownMenuItem key={step} onClick={() => onRerun(step)} className="max-md:min-h-11">
              {t('recordings.rerunStep', { step: t(`recordings.steps.${step}`) })}
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
