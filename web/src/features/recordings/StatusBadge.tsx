import { useTranslation } from 'react-i18next'
import type { RecordingItem } from './api'

// The glyphs of the terminal app: the shape tells the state apart without the color.
const STYLE = {
  done: ['●', 'text-success border-success/40 bg-success/10'],
  processing: ['◌', 'text-primary border-primary/40 bg-primary/10'],
  failed: ['▲', 'text-destructive border-destructive/40 bg-destructive/10'],
  noSpeech: ['○', 'text-muted-foreground border-border bg-background'],
} as const

export function StatusBadge({ recording }: { recording: Pick<RecordingItem, 'status' | 'no_speech'> }) {
  const { t } = useTranslation()
  const [glyph, style] = STYLE[recording.no_speech ? 'noSpeech' : recording.status]
  return (
    <span className={`inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs font-medium whitespace-nowrap ${style}`}>
      <span aria-hidden="true" className={recording.status === 'processing' ? 'motion-safe:animate-pulse' : ''}>
        {glyph}
      </span>
      {recording.no_speech ? t('recordings.noSpeech') : t(`recordings.status.${recording.status}`)}
    </span>
  )
}
