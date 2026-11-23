import { DownloadIcon, EllipsisIcon } from 'lucide-react'
import type { Ref } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'

const RERUN_STEPS = ['transcribe', 'summarize', 'diarize', 'export'] as const

/** The "more" menu of a recording: download the audio, run a step again (`disabled` locks the reruns only).
 *  `triggerRef` is the menu button, where the focus returns when a rerun's confirmation closes. */
export function RecordingMenu({
  id,
  disabled,
  onRerun,
  triggerRef,
}: {
  id: number
  disabled: boolean
  onRerun: (step: string) => void
  triggerRef: Ref<HTMLButtonElement>
}) {
  const { t } = useTranslation()
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            ref={triggerRef}
            variant="ghost"
            size="icon"
            aria-label={t('recordings.moreActions')}
            className="shrink-0 max-md:size-11"
          />
        }
      >
        <EllipsisIcon />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-auto">
        <DropdownMenuItem
          render={<a href={`/api/recordings/${id}/audio`} download />}
          className="max-md:min-h-11"
        >
          <DownloadIcon aria-hidden="true" />
          {t('recordings.downloadAudio')}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuLabel>{t('recordings.rerunHelp')}</DropdownMenuLabel>
          {RERUN_STEPS.map((step) => (
            <DropdownMenuItem key={step} disabled={disabled} onClick={() => onRerun(step)} className="max-md:min-h-11">
              {t('recordings.rerunStep', { step: t(`recordings.steps.${step}`) })}
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
