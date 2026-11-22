import i18n from '@/shared/i18n'

/** `created_at` is RFC3339 (`2026-11-07T10:00:00Z`): `Date` parses it natively. */
export function formatDate(rfc3339: string): string {
  return new Date(rfc3339).toLocaleString(i18n.language, { dateStyle: 'short', timeStyle: 'short' })
}

/** `75` → `01:15`, `3700` → `1:01:40`. */
export function formatTimestamp(seconds: number): string {
  const s = Math.floor(seconds)
  const pad = (n: number) => String(n).padStart(2, '0')
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  return h ? `${h}:${pad(m)}:${pad(s % 60)}` : `${pad(m)}:${pad(s % 60)}`
}
