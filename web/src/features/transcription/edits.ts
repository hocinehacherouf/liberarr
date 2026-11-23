import type { SegmentChange, Transcript } from '@/features/recordings/api'

/** The change setting unit `index` to `text`; `current` is what the page displays there: segments[index].text
 *  (`""` when removed), or t.text for the single unit of a transcript without timestamps. */
export function unitChange(t: Transcript, index: number, text: string): SegmentChange {
  return { index, current: t.has_timestamps ? (t.segments[index]?.text ?? '') : t.text, text }
}

/** Whether the transcript holds any correction: a segment with an original, or text_original set. */
export function hasEdits(t: Transcript | null): boolean {
  return t != null && (t.text_original != null || t.segments.some((s) => s.original != null))
}
