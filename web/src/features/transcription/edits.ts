import type { SegmentChange, Transcript } from '@/features/recordings/api'

/** The change setting unit `index` to `text`; `current` is what the page displays there: segments[index].text
 *  (`""` when removed), or t.text for the single unit of a transcript without timestamps. */
export function unitChange(t: Transcript, index: number, text: string): SegmentChange {
  return { index, current: t.has_timestamps ? (t.segments[index]?.text ?? '') : t.text, text }
}

/** One change per unit whose displayed text changes (`current` = that displayed text). Removed segments are skipped,
 *  and so is a unit whose result would be blank (removal stays explicit). Without timestamps the only unit is the text. */
export function findReplaceEdits(t: Transcript, find: string, replacement: string, matchCase: boolean): SegmentChange[] {
  if (find === '') return []
  // Literal, every occurrence; case-insensitive unless matchCase.
  const pattern = new RegExp(find.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), matchCase ? 'gu' : 'giu')
  const units = t.has_timestamps ? t.segments.map((s) => (s.removed ? null : s.text)) : [t.text]
  return units.flatMap((text, index) => {
    if (text == null) return []
    // A function, so the replacement is not expanded ($&, $1…).
    const next = text.replace(pattern, () => replacement)
    return next !== text && next.trim() !== '' ? [unitChange(t, index, next)] : []
  })
}

/** Whether the transcript holds any correction: a segment with an original, or text_original set. */
export function hasEdits(t: Transcript | null): boolean {
  return t != null && (t.text_original != null || t.segments.some((s) => s.original != null))
}
