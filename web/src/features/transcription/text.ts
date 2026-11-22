import { formatTimestamp } from '@/shared/time'
import type { Segment, Transcript } from '@/features/recordings/api'

/** "Alice: " before the text of a segment with a known speaker; `speaker` gives the display name. */
const who = (s: Segment, speaker: (n: number) => string) => (s.speaker == null ? '' : `${speaker(s.speaker)}: `)

/** Plain text to paste: one segment per line, or the raw text when the provider gave no timestamps. */
export function transcriptCopyText(t: Transcript, speaker: (n: number) => string): string {
  return t.has_timestamps ? t.segments.map((s) => who(s, speaker) + s.text).join('\n') : t.text
}

/** File content: `[12:34] text` per segment, or the raw text without timestamps. Ends with a newline. */
export function transcriptFileText(t: Transcript, speaker: (n: number) => string): string {
  const body = t.has_timestamps
    ? t.segments.map((s) => `[${formatTimestamp(s.start)}] ${who(s, speaker)}${s.text}`).join('\n')
    : t.text
  return `${body}\n`
}

/** `Design review: onboarding` → `Design review_ onboarding.txt`; `recording-<id>.txt` when nothing is left. */
export function transcriptFileName(title: string | null | undefined, id: number): string {
  const safe = [...(title ?? '').replace(/[\\/:*?"<>|\p{Cc}]/gu, '_').trim()].slice(0, 100).join('').trim()
  return `${safe || `recording-${id}`}.txt`
}

export function downloadText(text: string, fileName: string) {
  const url = URL.createObjectURL(new Blob([text], { type: 'text/plain;charset=utf-8' }))
  const a = document.createElement('a')
  a.href = url
  a.download = fileName
  a.click()
  // Not right away: some browsers start the download after the click returns.
  setTimeout(() => URL.revokeObjectURL(url), 10_000)
}
