import { useTranslation } from 'react-i18next'
import type { RecordingItem } from './api'

// The glyphs of the terminal app: the shape tells the state apart without the color.
const STYLE = {
  done: ['●', 'text-ok border-ok/40 bg-ok/10'],
  processing: ['◌', 'text-accent border-accent/40 bg-accent/10'],
  failed: ['▲', 'text-err border-err/40 bg-err/10'],
  noSpeech: ['○', 'text-muted border-line bg-canvas'],
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
