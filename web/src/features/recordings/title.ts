import type { TFunction } from 'i18next'
import type { RecordingItem } from './api'

export const recordingTitle = (r: Pick<RecordingItem, 'id' | 'title'>, t: TFunction): string =>
  r.title ?? t('recordings.untitled', { id: r.id })

export const statusLabel = (r: Pick<RecordingItem, 'status' | 'no_speech'>, t: TFunction): string =>
  r.no_speech ? t('recordings.noSpeech') : t(`recordings.status.${r.status}`)
