import { CircleCheckIcon, LoaderCircleIcon, MicOffIcon, TriangleAlertIcon, type LucideIcon } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Badge } from '@/components/ui/badge'
import type { RecordingItem } from './api'

type State = Pick<RecordingItem, 'status' | 'no_speech'>

// An icon per state: the shape tells the state apart without the color.
const BADGE = {
  done: [CircleCheckIcon, 'secondary', 'bg-success/10 text-success dark:bg-success/20'],
  processing: [LoaderCircleIcon, 'secondary', 'motion-safe:*:animate-spin'],
  failed: [TriangleAlertIcon, 'destructive', ''],
  noSpeech: [MicOffIcon, 'outline', ''],
} as const

const stateOf = (r: State) => (r.no_speech ? 'noSpeech' : r.status)

// oxlint-disable-next-line react/only-export-components -- the sidebar and the palette show the same icons
export const statusIcon = (r: State): LucideIcon => BADGE[stateOf(r)][0]

export function StatusBadge({ recording }: { recording: State }) {
  const { t } = useTranslation()
  const state = stateOf(recording)
  const [Icon, variant, className] = BADGE[state]
  return (
    <Badge variant={variant} className={className}>
      <Icon data-icon="inline-start" aria-hidden="true" />
      {t(state === 'noSpeech' ? 'recordings.noSpeech' : `recordings.status.${state}`)}
    </Badge>
  )
}
