import { ReplaceIcon } from 'lucide-react'
import { useMemo, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogTrigger } from '@/components/ui/dialog'
import { Field, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import type { SegmentChange, Transcript } from '@/features/recordings/api'
import { findReplaceEdits } from '@/features/transcription/edits'

// The API refuses a larger batch.
const MAX_EDITS = 5000

interface Props {
  transcript: Transcript
  /** A transcription is queued or running: the API would refuse an edit. */
  locked: boolean
  /** Saves the changes and shows the error itself; rejects when the save failed. */
  onEdit: (changes: SegmentChange[]) => Promise<void>
}

export function FindReplaceDialog({ transcript, locked, onEdit }: Props) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [find, setFind] = useState('')
  const [replacement, setReplacement] = useState('')
  const [matchCase, setMatchCase] = useState(false)
  const [pending, setPending] = useState(false)
  const edits = useMemo(() => findReplaceEdits(transcript, find, replacement, matchCase), [transcript, find, replacement, matchCase])
  const tooMany = edits.length > MAX_EDITS
  const canReplace = find !== '' && edits.length > 0 && !tooMany && !pending

  const onOpenChange = (next: boolean) => {
    if (pending) return
    if (next) {
      setFind('')
      setReplacement('')
      setMatchCase(false)
    }
    setOpen(next)
  }
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!canReplace) return
    setPending(true)
    try {
      await onEdit(edits)
      setOpen(false)
    } catch {
      // onEdit has already toasted; the dialog stays open.
    } finally {
      setPending(false)
    }
  }

  let status = ''
  if (find !== '') status = tooMany ? t('transcription.findTooMany', { max: MAX_EDITS }) : edits.length === 0 ? t('transcription.findNone') : t('transcription.findCount', { count: edits.length })

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogTrigger
        render={
          <Button type="button" variant="outline" size="sm" disabled={locked} className="max-md:h-11">
            <ReplaceIcon data-icon="inline-start" aria-hidden="true" />
            {t('transcription.findReplace')}
          </Button>
        }
      />
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t('transcription.findReplace')}</DialogTitle>
        </DialogHeader>
        <form onSubmit={submit} className="grid gap-6">
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="find-text">{t('transcription.find')}</FieldLabel>
              <Input id="find-text" autoFocus value={find} readOnly={pending} onChange={(e) => setFind(e.target.value)} className="max-md:h-11" />
            </Field>
            <Field>
              <FieldLabel htmlFor="replace-text">{t('transcription.replaceWith')}</FieldLabel>
              <Input id="replace-text" value={replacement} readOnly={pending} onChange={(e) => setReplacement(e.target.value)} className="max-md:h-11" />
            </Field>
            <Field orientation="horizontal">
              {/* Wrapped, not htmlFor: the Base UI checkbox is a button, not a native input. */}
              <FieldLabel>
                <Checkbox checked={matchCase} disabled={pending} onCheckedChange={setMatchCase} />
                {t('transcription.matchCase')}
              </FieldLabel>
            </Field>
          </FieldGroup>
          <p role="status" className="min-h-5 text-sm text-muted-foreground">
            {status}
          </p>
          <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
            <Button type="button" variant="outline" disabled={pending} onClick={() => onOpenChange(false)} className="max-md:h-11">
              {t('transcription.cancel')}
            </Button>
            <Button type="submit" disabled={!canReplace} className="max-md:h-11">
              {t('transcription.replace')}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}
