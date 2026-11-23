import { PencilIcon } from 'lucide-react'
import { memo, useCallback, useEffect, useRef, useState, type KeyboardEvent, type RefObject } from 'react'
import { useTranslation } from 'react-i18next'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { formatTimestamp } from '@/shared/time'
import type { Segment, SegmentChange, Transcript } from '@/features/recordings/api'
import { SpeakerLabel } from '@/features/transcription/SpeakerLabel'
import { unitChange } from '@/features/transcription/edits'

interface Props {
  transcript: Transcript
  /** Display name of a 0-based speaker. */
  speakerName: (n: number) => string
  onRename: (speaker: number, name: string) => Promise<unknown>
  /** Saves the changes and shows the error itself; rejects when the save failed. */
  onEdit: (changes: SegmentChange[]) => Promise<void>
  /** A transcription is queued or running: the API would refuse an edit. */
  locked: boolean
  audio: RefObject<HTMLAudioElement | null>
  onSeek: (seconds: number) => void
  /** Scrolls the segment at `time` into view once per `key` (the router's location key), and on mount. */
  focus?: { time: number; key: string }
}

/** Runs a save and tells whether it went through; the caller of `onEdit` has already toasted a failure. */
function useCommit() {
  const [pending, setPending] = useState(false)
  const commit = async (save: () => Promise<void>) => {
    setPending(true)
    try {
      await save()
      return true
    } catch {
      return false
    } finally {
      setPending(false)
    }
  }
  return { pending, commit }
}

/** "Edited" badge that shows the engine's text, with a button putting it back. */
function EditedLine({ original, locked, pending, onRevert }: { original: string; locked: boolean; pending: boolean; onRevert: () => void }) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <div className="flex basis-full flex-wrap items-center gap-2 text-sm">
      <Badge variant="secondary" render={<button type="button" aria-expanded={open} onClick={() => setOpen(!open)} />} className="cursor-pointer max-md:h-11">
        {t('transcription.edited')}
      </Badge>
      {open && (
        <>
          <span className="min-w-0 text-muted-foreground whitespace-pre-wrap">{t('transcription.original', { text: original })}</span>
          <Button type="button" variant="outline" size="xs" disabled={locked || pending} onClick={onRevert} className="max-md:h-11">
            {t('transcription.revert')}
          </Button>
        </>
      )}
    </div>
  )
}

// Memoised so a 4 Hz timeupdate re-renders only the rows whose `active` flag flips.
const Row = memo(function Row({
  s,
  index,
  active,
  label,
  editing,
  locked,
  onSeek,
  onRename,
  onChange,
  onStartEdit,
  onStopEdit,
}: {
  s: Segment
  index: number
  active: boolean
  /** The speaker's display name; null when the segment has no speaker. */
  label: string | null
  editing: boolean
  locked: boolean
  onSeek: (seconds: number) => void
  onRename: (speaker: number, name: string) => Promise<unknown>
  onChange: (index: number, text: string) => Promise<void>
  onStartEdit: (index: number) => void
  onStopEdit: () => void
}) {
  const { t } = useTranslation()
  const { pending, commit } = useCommit()
  const [draft, setDraft] = useState('')
  const pencil = useRef<HTMLButtonElement>(null)
  const restore = useRef<HTMLButtonElement>(null)
  // Where the focus goes once the row shows its new state: the opener, which is the Restore button after a removal.
  const refocus = useRef<'edit' | 'restore' | null>(null)
  useEffect(() => {
    if (editing || !refocus.current) return
    const el = (refocus.current === 'restore' ? restore : pencil).current
    if (!el) return
    el.focus()
    refocus.current = null
  }, [editing, s.removed, s.original])

  const change = async (text: string) => {
    if (locked || pending) return false
    const saved = await commit(() => onChange(index, text))
    if (saved) refocus.current = text.trim() === '' ? 'restore' : 'edit'
    return saved
  }
  // Blank is not a save: Remove is the explicit action, as in the whole-text editor.
  const blank = draft.trim() === ''
  const save = async (text: string) => {
    if (blank) return
    if (text === s.text) {
      // Nothing to save: just close, and hand the focus back like any other close.
      refocus.current = 'edit'
      onStopEdit()
    } else if (await change(text)) onStopEdit()
  }
  const cancel = () => {
    refocus.current = 'edit'
    onStopEdit()
  }
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Escape') {
      e.preventDefault()
      cancel()
    } else if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault()
      void save(draft)
    }
  }

  const time = formatTimestamp(s.start)
  // The label is a button of its own, so it cannot sit inside the seek button: the row is split around it.
  // Both seek buttons do the same; the text one is a mouse convenience, outside the tab order.
  return (
    <li
      className={`flex w-full flex-wrap gap-x-3 rounded-md border-l-2 px-2 py-1 leading-relaxed ${active ? 'border-primary bg-accent' : 'border-transparent hover:bg-accent/50'}`}
    >
      <Button
        type="button"
        variant="ghost"
        size="xs"
        onClick={() => onSeek(s.start)}
        aria-current={active || undefined}
        className="self-start font-mono tabular-nums"
      >
        {time}
      </Button>
      {s.speaker != null && label != null && <SpeakerLabel speaker={s.speaker} label={label} onRename={onRename} />}
      {s.removed ? (
        <div className="flex min-w-0 basis-full flex-wrap items-center gap-2 py-0.5 sm:flex-1 sm:basis-0">
          <span className="min-w-0 text-muted-foreground line-through">{s.original}</span>
          <span className="sr-only">{t('transcription.removedSegment')}</span>
          <Button
            ref={restore}
            type="button"
            variant="outline"
            size="xs"
            disabled={locked || pending}
            onClick={() => void change(s.original ?? '')}
            className="max-md:h-11"
          >
            {t('transcription.restore')}
          </Button>
        </div>
      ) : editing ? (
        <div className="min-w-0 basis-full space-y-2 py-0.5 sm:flex-1 sm:basis-0">
          <Textarea
            autoFocus
            value={draft}
            readOnly={pending}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={onKeyDown}
            aria-label={t('transcription.segmentText', { time })}
          />
          <div className="flex flex-wrap gap-2">
            <Button type="button" size="sm" disabled={blank || locked || pending} onClick={() => void save(draft)} className="max-md:h-11">
              {t('transcription.save')}
            </Button>
            <Button type="button" variant="outline" size="sm" disabled={pending} onClick={cancel} className="max-md:h-11">
              {t('transcription.cancel')}
            </Button>
            <Button
              type="button"
              variant="destructive"
              size="sm"
              disabled={locked || pending}
              onClick={() => void change('').then((saved) => saved && onStopEdit())}
              className="max-md:h-11 sm:ml-auto"
            >
              {t('transcription.remove')}
            </Button>
          </div>
        </div>
      ) : (
        <>
          <button type="button" tabIndex={-1} onClick={() => onSeek(s.start)} className="min-w-0 basis-full py-0.5 text-left sm:flex-1 sm:basis-0">
            {s.text}
          </button>
          <Button
            ref={pencil}
            type="button"
            variant="ghost"
            size="icon-xs"
            aria-label={t('transcription.edit')}
            disabled={locked}
            onClick={() => {
              setDraft(s.text)
              onStartEdit(index)
            }}
            className="self-start max-md:size-11"
          >
            <PencilIcon aria-hidden="true" />
          </Button>
          {s.original != null && (
            <EditedLine original={s.original} locked={locked} pending={pending} onRevert={() => void change(s.original ?? '')} />
          )}
        </>
      )}
    </li>
  )
})

/** A transcript without timestamps is one unit: the whole text. */
function WholeText({ transcript, locked, onEdit }: { transcript: Transcript; locked: boolean; onEdit: Props['onEdit'] }) {
  const { t } = useTranslation()
  const { pending, commit } = useCommit()
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const opener = useRef<HTMLButtonElement>(null)
  const wasEditing = useRef(false)
  // Hand the focus back to the Edit button when the editor closes.
  useEffect(() => {
    if (wasEditing.current && !editing) opener.current?.focus()
    wasEditing.current = editing
  }, [editing])

  const blank = draft.trim() === ''
  const change = (text: string) => commit(() => onEdit([unitChange(transcript, 0, text)]))
  const save = async () => {
    if (blank || locked || pending) return
    if (draft === transcript.text || (await change(draft))) setEditing(false)
  }
  const original = transcript.text_original

  return (
    <div className="space-y-3">
      <p className="text-sm text-muted-foreground">{t('transcription.noTimestamps')}</p>
      {editing ? (
        <div className="space-y-2">
          <Textarea
            autoFocus
            value={draft}
            readOnly={pending}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') {
                e.preventDefault()
                setEditing(false)
              } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                e.preventDefault()
                void save()
              }
            }}
            aria-label={t('transcription.wholeText')}
          />
          <div className="flex flex-wrap gap-2">
            <Button type="button" size="sm" disabled={blank || locked || pending} onClick={() => void save()} className="max-md:h-11">
              {t('transcription.save')}
            </Button>
            <Button type="button" variant="outline" size="sm" disabled={pending} onClick={() => setEditing(false)} className="max-md:h-11">
              {t('transcription.cancel')}
            </Button>
          </div>
        </div>
      ) : (
        <>
          <Button
            ref={opener}
            type="button"
            variant="outline"
            size="sm"
            disabled={locked}
            onClick={() => {
              setDraft(transcript.text)
              setEditing(true)
            }}
            className="max-md:h-11"
          >
            <PencilIcon data-icon="inline-start" aria-hidden="true" />
            {t('transcription.editWhole')}
          </Button>
          <p className="leading-relaxed whitespace-pre-wrap">{transcript.text}</p>
          {original != null && (
            <EditedLine original={original} locked={locked} pending={pending} onRevert={() => void change(original)} />
          )}
        </>
      )}
    </div>
  )
}

export function TranscriptView({ transcript, speakerName, onRename, onEdit, locked, audio, onSeek, focus }: Props) {
  // The playback time lives here, not in the page, so the summary and mind map never re-render on it.
  const [time, setTime] = useState(0)
  const [editing, setEditing] = useState<number | null>(null)
  useEffect(() => {
    const el = audio.current
    if (!el) return
    const update = () => setTime(el.currentTime)
    el.addEventListener('timeupdate', update)
    return () => el.removeEventListener('timeupdate', update)
  }, [audio])
  const onChange = useCallback((index: number, text: string) => onEdit([unitChange(transcript, index, text)]), [onEdit, transcript])
  const stopEdit = useCallback(() => setEditing(null), [])
  const list = useRef<HTMLOListElement>(null)
  useEffect(() => {
    if (!focus || !transcript.has_timestamps) return
    // Default (instant) scrolling: reduced motion is respected.
    list.current?.children[transcript.segments.findLastIndex((s) => s.start <= focus.time)]?.scrollIntoView({ block: 'center' })
    // Only a new jump scrolls: a refetched transcript (an edit, a poll) must not.
    // oxlint-disable-next-line react-hooks/exhaustive-deps -- keyed on focus.key alone, as said above
  }, [focus?.key])

  if (!transcript.has_timestamps) return <WholeText transcript={transcript} locked={locked} onEdit={onEdit} />

  return (
    <ol ref={list} className="-mx-2 space-y-0.5">
      {transcript.segments.map((s, i) => (
        <Row
          key={i}
          s={s}
          index={i}
          active={!s.removed && time >= s.start && time < s.end}
          label={s.speaker == null ? null : speakerName(s.speaker)}
          editing={editing === i}
          locked={locked}
          onSeek={onSeek}
          onRename={onRename}
          onChange={onChange}
          onStartEdit={setEditing}
          onStopEdit={stopEdit}
        />
      ))}
    </ol>
  )
}
