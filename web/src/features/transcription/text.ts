import { formatTimestamp } from '../../shared/time'
import type { Transcript } from '../recordings/api'

/** Plain text to paste: one segment per line, or the raw text when the provider gave no timestamps. */
export function transcriptCopyText(t: Transcript): string {
  return t.has_timestamps ? t.segments.map((s) => s.text).join('\n') : t.text
}

/** File content: `[12:34] text` per segment, or the raw text without timestamps. Ends with a newline. */
export function transcriptFileText(t: Transcript): string {
  const body = t.has_timestamps ? t.segments.map((s) => `[${formatTimestamp(s.start)}] ${s.text}`).join('\n') : t.text
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
