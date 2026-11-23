import type { Job } from '@/features/recordings/api'

const active = (jobs: Job[]) => jobs.filter((j) => j.status === 'queued' || j.status === 'running')

/** Whether a job of `step` is queued or running. */
export const stepBusy = (jobs: Job[], step: string) => active(jobs).some((j) => j.step === step)

/** True when the recording has queued or running jobs and every one of them is `export` (a save queues one). */
export function exportOnly(jobs: Job[]): boolean {
  const a = active(jobs)
  return a.length > 0 && a.every((j) => j.step === 'export')
}
