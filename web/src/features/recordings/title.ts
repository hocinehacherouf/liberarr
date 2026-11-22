import type { TFunction } from 'i18next'
import type { RecordingItem } from './api'

export const recordingTitle = (r: Pick<RecordingItem, 'id' | 'title'>, t: TFunction): string =>
  r.title ?? t('recordings.untitled', { id: r.id })
