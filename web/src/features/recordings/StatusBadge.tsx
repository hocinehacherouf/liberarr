import { CircleCheckIcon, LoaderCircleIcon, MicOffIcon, TriangleAlertIcon } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Badge } from '@/components/ui/badge'
import type { RecordingItem } from './api'

type State = Pick<RecordingItem, 'status' | 'no_speech'>

// An icon per state: the shape tells the state apart without the color.
const STATES = {
  done: {
    Icon: CircleCheckIcon,
    variant: 'secondary',
    badgeClass: 'bg-success/10 text-success dark:bg-success/20',
    iconClass: '',
    label: 'recordings.status.done',
  },
  processing: {
    Icon: LoaderCircleIcon,
    variant: 'secondary',
    badgeClass: 'motion-safe:*:animate-spin',
    iconClass: 'motion-safe:animate-spin',
    label: 'recordings.status.processing',
  },
  failed: {
    Icon: TriangleAlertIcon,
    variant: 'destructive',
    badgeClass: '',
    iconClass: 'text-destructive',
    label: 'recordings.status.failed',
  },
  noSpeech: { Icon: MicOffIcon, variant: 'outline', badgeClass: '', iconClass: '', label: 'recordings.noSpeech' },
} as const

// The one place that decides the state (no speech wins over the job status); the sidebar and the palette reuse it.
// oxlint-disable-next-line react/only-export-components -- not a component
export const recordingStatus = (r: State) => STATES[r.no_speech ? 'noSpeech' : r.status]

export function StatusBadge({ recording }: { recording: State }) {
  const { t } = useTranslation()
  const { Icon, variant, badgeClass, label } = recordingStatus(recording)
  return (
    <Badge variant={variant} className={badgeClass}>
      <Icon data-icon="inline-start" aria-hidden="true" />
      {t(label)}
    </Badge>
  )
}
