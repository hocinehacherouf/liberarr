import { memo, useEffect, useState, type RefObject } from 'react'
import { useTranslation } from 'react-i18next'
import { formatTimestamp } from '../../shared/time'
import type { Segment, Transcript } from '../recordings/api'
import { SpeakerLabel } from './SpeakerLabel'

interface Props {
  transcript: Transcript
  /** Display name of a 0-based speaker. */
  speakerName: (n: number) => string
  onRename: (speaker: number, name: string) => Promise<unknown>
  audio: RefObject<HTMLAudioElement | null>
  onSeek: (seconds: number) => void
}

// Memoised so a 4 Hz timeupdate re-renders only the rows whose `active` flag flips.
const Row = memo(function Row({
  s,
  active,
  label,
  onSeek,
  onRename,
}: {
  s: Segment
  active: boolean
  /** The speaker's display name; null when the segment has no speaker. */
  label: string | null
  onSeek: (seconds: number) => void
  onRename: (speaker: number, name: string) => Promise<unknown>
}) {
  // The label is a button of its own, so it cannot sit inside the seek button: the row is split around it.
  // Both seek buttons do the same; the text one is a mouse convenience, outside the tab order.
  return (
    <li
      className={`flex w-full flex-wrap gap-x-3 rounded-md border-l-2 px-2 py-1.5 leading-relaxed hover:bg-accent/5 ${active ? 'border-accent bg-accent/10' : 'border-transparent'}`}
    >
      <button
        type="button"
        onClick={() => onSeek(s.start)}
        aria-current={active || undefined}
        className="self-start pt-0.5 font-mono text-xs text-accent tabular-nums"
      >
        {formatTimestamp(s.start)}
      </button>
      {s.speaker != null && label != null && <SpeakerLabel speaker={s.speaker} label={label} onRename={onRename} />}
      <button type="button" tabIndex={-1} onClick={() => onSeek(s.start)} className="min-w-0 flex-1 text-left">
        {s.text}
      </button>
    </li>
  )
})

export function TranscriptView({ transcript, speakerName, onRename, audio, onSeek }: Props) {
  const { t } = useTranslation()
  // The playback time lives here, not in the page, so the summary and mind map never re-render on it.
  const [time, setTime] = useState(0)
  useEffect(() => {
    const el = audio.current
    if (!el) return
    const update = () => setTime(el.currentTime)
    el.addEventListener('timeupdate', update)
    return () => el.removeEventListener('timeupdate', update)
  }, [audio])

  if (!transcript.has_timestamps)
    return (
      <div className="space-y-3">
        <p className="text-sm text-muted">{t('transcription.noTimestamps')}</p>
        <p className="leading-relaxed whitespace-pre-wrap">{transcript.text}</p>
      </div>
    )

  return (
    <ol className="-mx-2 space-y-0.5">
      {transcript.segments.map((s, i) => (
        <Row
          key={i}
          s={s}
          active={time >= s.start && time < s.end}
          label={s.speaker == null ? null : speakerName(s.speaker)}
          onSeek={onSeek}
          onRename={onRename}
        />
      ))}
    </ol>
  )
}
