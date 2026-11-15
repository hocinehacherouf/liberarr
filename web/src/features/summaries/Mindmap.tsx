import { useEffect, useId, useState, useSyncExternalStore } from 'react'
import { useTranslation } from 'react-i18next'

const darkQuery = window.matchMedia('(prefers-color-scheme: dark)')
const subscribe = (onChange: () => void) => {
  darkQuery.addEventListener('change', onChange)
  return () => darkQuery.removeEventListener('change', onChange)
}

// Branches alternate the brand's two ends; the root is the darker cyan, readable under white text.
const brandVariables = (dark: boolean) => ({
  fontFamily: 'inherit',
  primaryColor: '#006d89',
  primaryTextColor: '#ffffff',
  lineColor: dark ? '#9198a1' : '#57606a',
  // The root node takes the git0 colors.
  git0: '#006d89',
  gitBranchLabel0: '#ffffff',
  ...Object.fromEntries(
    Array.from({ length: 12 }, (_, i) => [
      [`cScale${i}`, i % 2 ? (dark ? '#4a3480' : '#e9e0fb') : dark ? '#0d4f61' : '#d4eef5'],
      [`cScaleLabel${i}`, dark ? '#e6edf3' : '#1f2328'],
    ]).flat(),
  ),
})

export function Mindmap({ code }: { code: string }) {
  const { t } = useTranslation()
  const id = `mm-${useId().replace(/[^a-zA-Z0-9]/g, '')}`
  const dark = useSyncExternalStore(subscribe, () => darkQuery.matches)
  const [svg, setSvg] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let alive = true
    // Loaded on demand: mermaid is heavy and only the detail page needs it.
    import('mermaid')
      .then(({ default: mermaid }) => {
        // strict: the diagram comes from an LLM, so no scripts and no raw HTML.
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: 'strict',
          theme: 'base',
          darkMode: dark,
          themeVariables: brandVariables(dark),
          suppressErrorRendering: true,
        })
        return mermaid.render(id, code)
      })
      .then((r) => alive && (setSvg(r.svg), setError(null)))
      .catch((e: unknown) => alive && (setSvg(null), setError(e instanceof Error ? e.message : String(e))))
    return () => {
      alive = false
    }
  }, [code, id, dark])

  if (error)
    return (
      <div className="space-y-2">
        <p role="alert" className="text-sm text-warn">
          {t('summaries.mindmapError', { error })}
        </p>
        <pre className="overflow-x-auto rounded-md border border-line bg-canvas p-3 text-xs">{code}</pre>
      </div>
    )
  return svg ? <div className="overflow-x-auto [&_svg]:mx-auto [&_svg]:min-w-[40rem]" dangerouslySetInnerHTML={{ __html: svg }} /> : null
}
