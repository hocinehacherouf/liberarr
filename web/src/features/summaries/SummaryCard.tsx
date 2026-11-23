import { PencilIcon } from 'lucide-react'
import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import Markdown from 'react-markdown'
import { toast } from 'sonner'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardAction, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Field, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { useEditSummary, type Summary } from '@/features/recordings/api'
import { ApiError } from '@/shared/api'
import { CopyButton } from '@/shared/CopyButton'

// The API refuses a longer title.
const MAX_TITLE = 200

// No raw HTML (react-markdown default) and no images: a remote URL in LLM output would leak.
const PROSE =
  'space-y-2 text-base leading-relaxed [&_a]:text-primary [&_a]:underline [&_h1]:mt-4 [&_h1]:font-semibold [&_h1]:text-primary [&_h1:first-child]:mt-0 [&_h2]:mt-4 [&_h2]:font-semibold [&_h2]:text-primary [&_h2:first-child]:mt-0 [&_h3]:mt-3 [&_h3]:font-semibold [&_li]:ml-5 [&_li::marker]:text-muted-foreground [&_ol]:list-decimal [&_ul]:list-disc'

interface Props {
  recordingId: number
  summary: Summary
  /** A transcription or summary is queued or running: the API would refuse an edit. */
  locked: boolean
}

/** The Summary tab card: the Markdown with Copy, and an editor for the title and text. */
export function SummaryCard({ recordingId, summary, locked }: Props) {
  const { t } = useTranslation()
  const { mutateAsync } = useEditSummary(recordingId)
  const [editing, setEditing] = useState(false)
  const [title, setTitle] = useState('')
  const [markdown, setMarkdown] = useState('')
  const [pending, setPending] = useState(false)
  const [restoreOpen, setRestoreOpen] = useState(false)
  const opener = useRef<HTMLButtonElement>(null)
  const wasEditing = useRef(false)
  // What the card showed when the editor opened: the server refuses the save when it has changed since.
  const shownAtOpen = useRef(summary)
  // Hand the focus back to the Edit button when the editor closes.
  useEffect(() => {
    if (wasEditing.current && !editing) opener.current?.focus()
    wasEditing.current = editing
  }, [editing])

  const generated = summary.generated
  // Toasts its own errors and rethrows: the form and the dialog close only when it resolves and never toast again.
  const save = async (next: { title: string; summary_md: string }, shown: { title: string; summary_md: string }) => {
    const from = generated ?? summary
    setPending(true)
    try {
      await mutateAsync({
        ...next,
        generated_title: from.title,
        generated_summary_md: from.summary_md,
        current_title: shown.title,
        current_summary_md: shown.summary_md,
      })
    } catch (e) {
      toast.error(t(e instanceof ApiError && e.status === 409 ? 'summaries.editStale' : 'summaries.editError'))
      throw e
    } finally {
      setPending(false)
    }
  }

  const unchanged = title === summary.title && markdown === summary.summary_md
  const canSave = title.trim() !== '' && !unchanged && !locked && !pending
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!canSave) return
    try {
      await save({ title, summary_md: markdown }, shownAtOpen.current)
      setEditing(false)
    } catch {
      // save has already toasted; the form stays open.
    }
  }
  const cancel = () => {
    if (!pending) setEditing(false)
  }
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== 'Escape') return
    e.preventDefault()
    cancel()
  }
  const restore = async () => {
    if (!generated) return
    try {
      await save(generated, summary)
      setRestoreOpen(false)
      // The Restore button is gone once the page shows the generated text.
      opener.current?.focus()
    } catch {
      // save has already toasted; the dialog stays open.
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-2">
          {t('summaries.summary')}
          {generated && <Badge variant="secondary">{t('summaries.edited')}</Badge>}
        </CardTitle>
        {!editing && (
          <CardAction className="flex gap-2">
            <Button
              ref={opener}
              type="button"
              variant="outline"
              aria-label={t('summaries.edit')}
              disabled={locked}
              onClick={() => {
                shownAtOpen.current = summary
                setTitle(summary.title)
                setMarkdown(summary.summary_md)
                setEditing(true)
              }}
              className="max-md:h-11"
            >
              <PencilIcon aria-hidden="true" />
              {t('summaries.editShort')}
            </Button>
            <CopyButton text={summary.summary_md} label={t('summaries.copy')} />
          </CardAction>
        )}
      </CardHeader>
      <CardContent className="space-y-4">
        {editing ? (
          <form onSubmit={submit} onKeyDown={onKeyDown} className="space-y-4">
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="summary-title">{t('summaries.titleLabel')}</FieldLabel>
                <Input
                  id="summary-title"
                  autoFocus
                  required
                  maxLength={MAX_TITLE}
                  value={title}
                  readOnly={pending}
                  onChange={(e) => setTitle(e.target.value)}
                  className="max-md:h-11"
                />
              </Field>
              <div className="grid gap-4 lg:grid-cols-2">
                <Field>
                  <FieldLabel htmlFor="summary-markdown">{t('summaries.markdownLabel')}</FieldLabel>
                  <Textarea
                    id="summary-markdown"
                    value={markdown}
                    readOnly={pending}
                    onChange={(e) => setMarkdown(e.target.value)}
                    className="min-h-64"
                  />
                </Field>
                <div role="region" aria-labelledby="summary-preview" className="space-y-2">
                  <p id="summary-preview" className="text-sm font-medium">
                    {t('summaries.preview')}
                  </p>
                  <div className={PROSE}>
                    <Markdown disallowedElements={['img']}>{markdown}</Markdown>
                  </div>
                </div>
              </div>
            </FieldGroup>
            <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
              <Button type="button" variant="outline" disabled={pending} onClick={cancel} className="max-md:h-11">
                {t('common.cancel')}
              </Button>
              <Button type="submit" disabled={!canSave} className="max-md:h-11">
                {t('common.save')}
              </Button>
            </div>
          </form>
        ) : (
          <>
            <div className={PROSE}>
              <Markdown disallowedElements={['img']}>{summary.summary_md}</Markdown>
            </div>
            {generated && (
              <AlertDialog open={restoreOpen} onOpenChange={(open) => !pending && setRestoreOpen(open)}>
                <AlertDialogTrigger
                  render={
                    <Button type="button" variant="outline" size="sm" disabled={locked} className="max-md:h-11">
                      {t('summaries.restore')}
                    </Button>
                  }
                />
                <AlertDialogContent>
                  <AlertDialogHeader>
                    <AlertDialogTitle>{t('summaries.restoreTitle')}</AlertDialogTitle>
                    <AlertDialogDescription>{t('summaries.restoreDescription')}</AlertDialogDescription>
                  </AlertDialogHeader>
                  <AlertDialogFooter>
                    <AlertDialogCancel disabled={pending} className="max-md:h-11">
                      {t('common.cancel')}
                    </AlertDialogCancel>
                    <AlertDialogAction disabled={locked || pending} onClick={() => void restore()} className="max-md:h-11">
                      {t('summaries.restoreConfirm')}
                    </AlertDialogAction>
                  </AlertDialogFooter>
                </AlertDialogContent>
              </AlertDialog>
            )}
          </>
        )}
      </CardContent>
    </Card>
  )
}
