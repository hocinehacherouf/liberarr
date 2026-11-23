import { useState, type JSX } from 'react'
import { useTranslation } from 'react-i18next'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import type { RecordingDetail } from '@/features/recordings/api'
import { hasEdits } from '@/features/transcription/edits'

/** Wraps a rerun: asks first when it would drop edits, else runs it at once. Render `dialog` once in the page.
 *  `rec` is undefined while the page loads; `request` is only called with one. */
export function useRerunConfirm(
  rec: RecordingDetail | undefined,
  run: (step: string) => void,
): { request: (step: string) => void; dialog: JSX.Element } {
  const { t } = useTranslation()
  // The step outlives `open`, so the text does not change while the dialog fades out.
  const [step, setStep] = useState('')
  const [open, setOpen] = useState(false)

  // What a rerun of `s` may drop: a new transcript can replace both overlays, a new summary only the summary's.
  const lostFor = (s: string) => [
    ...(s === 'transcribe' && hasEdits(rec?.transcript ?? null) ? ['transcript'] : []),
    ...((s === 'transcribe' || s === 'summarize') && rec?.summary?.generated ? ['summary'] : []),
  ]
  const lost = lostFor(step)

  const request = (s: string) => {
    if (lostFor(s).length === 0) return run(s)
    setStep(s)
    setOpen(true)
  }

  const dialog = (
    <AlertDialog open={open} onOpenChange={setOpen}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t('recordings.rerunConfirm.title', { step: t(`recordings.steps.${step}`, { defaultValue: step }) })}</AlertDialogTitle>
          <AlertDialogDescription render={<ul className="list-disc space-y-1 pl-5 text-left" />}>
            {lost.map((item) => (
              <li key={item}>{t(`recordings.rerunConfirm.${item}`)}</li>
            ))}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel className="max-md:h-11">{t('common.cancel')}</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            onClick={() => {
              setOpen(false)
              run(step)
            }}
            className="max-md:h-11"
          >
            {t('recordings.rerunConfirm.confirm')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
  return { request, dialog }
}
