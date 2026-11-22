import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api, ApiError } from '../../shared/api'

export type Status = 'processing' | 'failed' | 'done'

export interface RecordingItem {
  id: number
  title: string | null
  source: 'device' | 'file'
  duration_s: number | null
  created_at: string
  status: Status
  failed_step: string | null
  no_speech: boolean
}

export interface Segment {
  start: number
  end: number
  text: string
  /** 0-based; null without a diarization or where no speaker covers the segment. */
  speaker: number | null
}

export interface Transcript {
  engine: string
  language: string | null
  text: string
  has_timestamps: boolean
  segments: Segment[]
}

export interface Action {
  action: string
  owner: string | null
  due: string | null
  start: number | null
  done: boolean
}

export interface Summary {
  title: string
  summary_md: string
  mindmap: string
  actions: Action[]
}

export interface Job {
  id: number
  step: string
  status: 'queued' | 'running' | 'failed' | 'done'
  attempts: number
  error: string | null
  updated_at: string
}

export interface Speakers {
  turns_present: boolean
  requested: number | null
  /** By 0-based speaker (JSON keys are strings). */
  names: Record<string, string>
}

export interface RecordingDetail extends RecordingItem {
  transcript: Transcript | null
  speakers: Speakers
  summary: Summary | null
  jobs: Job[]
}

const POLL_MS = 5000

export const useRecordings = () =>
  useQuery({
    queryKey: ['recordings'],
    queryFn: () => api<RecordingItem[]>('/api/recordings'),
    refetchInterval: (q) => (q.state.data?.some((r) => r.status === 'processing') ? POLL_MS : false),
  })

export const useRecording = (id: number) =>
  useQuery({
    queryKey: ['recordings', id],
    queryFn: () => api<RecordingDetail>(`/api/recordings/${id}`),
    refetchInterval: (q) => (q.state.data?.status === 'processing' ? POLL_MS : false),
  })

export function useRerun(id: number) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (step: string) => api(`/api/recordings/${id}/rerun`, { method: 'POST', body: JSON.stringify({ step }) }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['recordings'] }),
  })
}

export function useRenameSpeaker(id: number) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ speaker, name }: { speaker: number; name: string }) =>
      api(`/api/recordings/${id}/speakers/${speaker}`, { method: 'PUT', body: JSON.stringify({ name }) }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['recordings', id] }),
  })
}

/** Saves the count and diarizes again (202): the page then shows the job like any rerun. */
export function useSpeakerCount(id: number) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (count: number | null) =>
      api(`/api/recordings/${id}/speaker-count`, { method: 'PUT', body: JSON.stringify({ count }) }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['recordings'] }),
  })
}

export function useToggleAction(id: number) {
  const qc = useQueryClient()
  return useMutation({
    // `action` is the text shown at `index`: the server answers 409 when a rerun has replaced it.
    mutationFn: ({ index, action, done }: { index: number; action: string; done: boolean }) =>
      api<Action[]>(`/api/recordings/${id}/actions/${index}`, {
        method: 'PATCH',
        body: JSON.stringify({ action, done }),
      }),
    // The PATCH returns the whole updated array: put it straight into the cached detail.
    onSuccess: (actions) =>
      qc.setQueryData<RecordingDetail>(['recordings', id], (d) =>
        d?.summary ? { ...d, summary: { ...d.summary, actions } } : d,
      ),
    onError: (e) => {
      if (e instanceof ApiError && e.status === 409) void qc.invalidateQueries({ queryKey: ['recordings', id] })
    },
  })
}
