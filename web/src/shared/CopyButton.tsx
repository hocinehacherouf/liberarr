import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

/** Copies `text` to the clipboard. `label` is the accessible name (what is copied). */
export function CopyButton({ text, label }: { text: string; label: string }) {
  const { t } = useTranslation()
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle')
  const timer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(timer.current), [])

  const copy = async () => {
    let next: 'copied' | 'failed' = 'copied'
    try {
      // `navigator.clipboard` is undefined outside a secure context (plain HTTP on the LAN).
      if (!navigator.clipboard) throw new Error('no clipboard')
      await navigator.clipboard.writeText(text)
    } catch {
      next = 'failed'
    }
    setState(next)
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => setState('idle'), next === 'copied' ? 2000 : 5000)
  }

  return (
    <>
      <button
        type="button"
        aria-label={label}
        onClick={() => void copy()}
        className="rounded-md border border-line bg-surface px-3 py-1 text-sm font-semibold hover:text-accent"
      >
        {t(state === 'copied' ? 'common.copied' : 'common.copy')}
      </button>
      {/* Always rendered, so screen readers pick up the change. */}
      <span aria-live="polite" className={state === 'failed' ? 'text-sm text-err' : 'sr-only'}>
        {state === 'idle' ? '' : t(state === 'copied' ? 'common.copied' : 'common.copyFailed')}
      </span>
    </>
  )
}
