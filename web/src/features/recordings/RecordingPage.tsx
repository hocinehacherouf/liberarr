import { useCallback, useRef, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import Markdown from 'react-markdown'
import { Link, useParams } from 'react-router'
import { Loading } from '../../shared/Loading'
import { ApiError } from '../../shared/api'
import { CopyButton } from '../../shared/CopyButton'
import { formatDate, formatTimestamp } from '../../shared/time'
import { ActionsTable } from '../summaries/ActionsTable'
import { Mindmap } from '../summaries/Mindmap'
import { TranscriptView } from '../transcription/TranscriptView'
import { downloadText, transcriptCopyText, transcriptFileName, transcriptFileText } from '../transcription/text'
import { useRecording, useRenameSpeaker, useRerun, useSpeakerCount, type Job } from './api'
import { StatusBadge } from './StatusBadge'
import { recordingTitle } from './title'

// Glyph and color of a pipeline step, as in the terminal app: the glyph alone tells the state.
const JOB: Record<Job['status'], [string, string]> = {
  done: ['✓', 'text-success'],
  failed: ['✕', 'text-destructive'],
  running: ['◌', 'text-primary motion-safe:animate-pulse'],
  queued: ['·', 'text-muted-foreground'],
}
const RERUN_STEPS = ['transcribe', 'summarize', 'diarize', 'export'] as const
const MAX_SPEAKER_CHOICE = 10

/** A pane of the page: rounded border, its title on top. */
function Pane({ title, actions, children }: { title: string; actions?: ReactNode; children: ReactNode }) {
  return (
    <section className="rounded-xl border border-border bg-card p-4 sm:p-5">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-sm font-semibold tracking-wide text-primary uppercase">{title}</h2>
        {actions && <div className="flex items-center gap-2">{actions}</div>}
      </div>
      {children}
    </section>
  )
}

/** Keyed by id: no state (errors, playback time, open forms) carries over from one recording to the next. */
export function RecordingPage() {
  const id = Number(useParams().id)
  return <RecordingContent key={id} id={id} />
}

function RecordingContent({ id }: { id: number }) {
  const { t } = useTranslation()
  const { data: rec, isLoading, isError } = useRecording(id)
  const rerun = useRerun(id)
  const speakerCount = useSpeakerCount(id)
  const { mutateAsync: renameSpeaker } = useRenameSpeaker(id)
  // Stable, so the memoised transcript rows do not re-render with the page.
  const onRename = useCallback((speaker: number, name: string) => renameSpeaker({ speaker, name }), [renameSpeaker])
  const audio = useRef<HTMLAudioElement>(null)

  const seek = (seconds: number) => {
    if (!audio.current) return
    audio.current.currentTime = seconds
    void audio.current.play().catch(() => {})
  }

  const back = (
    <Link to="/" className="rounded text-sm text-muted-foreground underline-offset-2 hover:text-foreground hover:underline">
      {t('recordings.back')}
    </Link>
  )
  if (isLoading) return <Loading className="mx-auto max-w-5xl px-4 py-8" />
  if (isError || !rec)
    return (
      <div className="mx-auto w-full max-w-5xl space-y-4 px-4 py-8">
        {back}
        <p role="alert" className="rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm text-destructive">
          {t('recordings.notFound')}
        </p>
      </div>
    )

  const failedStep = rec.status === 'failed' ? rec.failed_step : null
  const failedJob = failedStep ? rec.jobs.findLast((j) => j.step === failedStep) : undefined
  const rerunDetail = rerun.error instanceof ApiError ? rerun.error.message : ''
  // The server refuses a summary rerun (409) when `liberarr init` chose no engine.
  const noEngine = rerun.error instanceof ApiError && rerun.error.status === 409 && rerun.variables === 'summarize'
  // 409: a step is running, or speaker detection is off.
  const countRefused = speakerCount.error instanceof ApiError && speakerCount.error.status === 409
  const requested = rec.speakers.requested
  const speakerName = (n: number) => rec.speakers.names[n] ?? t('speakers.label', { n: n + 1 })
  // The latest job of each step, in the order the steps first ran.
  const steps = [...new Set(rec.jobs.map((j) => j.step))].map((step) => rec.jobs.findLast((j) => j.step === step)!)

  return (
    <div className="mx-auto w-full max-w-5xl space-y-4 px-4 py-6 sm:py-8">
      <div className="space-y-2">
        {back}
        <h1 className="text-2xl font-semibold tracking-tight text-balance">
          {rec.summary?.title ?? recordingTitle(rec, t)}
        </h1>
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2 text-sm text-muted-foreground">
          <StatusBadge recording={rec} />
          <time dateTime={rec.created_at}>{formatDate(rec.created_at)}</time>
          {rec.duration_s != null && <span className="font-mono tabular-nums">{formatTimestamp(rec.duration_s)}</span>}
          {steps.length > 0 && (
            <ol aria-label={t('recordings.pipeline')} className="flex flex-wrap items-center gap-x-2 font-mono text-xs">
              {steps.map((j, i) => (
                <li key={j.step} className="flex items-center gap-2">
                  {i > 0 && <span aria-hidden="true" className="text-border">──</span>}
                  <span className={JOB[j.status][1]} aria-hidden="true">
                    {JOB[j.status][0]}
                  </span>
                  <span className="text-foreground">{t(`recordings.steps.${j.step}`, { defaultValue: j.step })}</span>
                  <span className="sr-only">{t(`recordings.jobStatus.${j.status}`)}</span>
                </li>
              ))}
            </ol>
          )}
        </div>
      </div>

      {failedStep && (
        <div
          role="alert"
          className="flex flex-wrap items-center gap-3 rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm"
        >
          <span aria-hidden="true" className="text-destructive">
            ▲
          </span>
          <span className="flex-1">
            {t(failedJob?.error ? 'recordings.stepFailedWithError' : 'recordings.stepFailed', {
              step: t(`recordings.steps.${failedStep}`, { defaultValue: failedStep }),
              error: failedJob?.error,
            })}
          </span>
          <button
            onClick={() => rerun.mutate(failedStep)}
            disabled={rerun.isPending}
            className="bg-brand-strong rounded-md px-3 py-1.5 font-semibold text-white hover:brightness-110 disabled:opacity-60"
          >
            {t('recordings.retry')}
          </button>
        </div>
      )}
      {rec.status !== 'processing' && (
        <fieldset disabled={rerun.isPending} aria-describedby="rerun-help" className="min-w-0 space-y-2">
          <legend className="font-semibold">{t('recordings.rerunTitle')}</legend>
          <p id="rerun-help" className="text-sm text-muted-foreground">{t('recordings.rerunHelp')}</p>
          <div className="flex flex-wrap gap-2">
            {RERUN_STEPS.map((step) => (
              <button
                key={step}
                type="button"
                onClick={() => rerun.mutate(step)}
                className="min-h-11 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-semibold hover:text-primary disabled:opacity-60"
              >
                {t('recordings.rerunStep', { step: t(`recordings.steps.${step}`) })}
              </button>
            ))}
          </div>
        </fieldset>
      )}
      {rerun.isError && (
        <p role="alert" className="text-sm text-destructive">
          {noEngine
            ? t('recordings.noSummaryEngine')
            : t(rerunDetail ? 'recordings.retryErrorWithDetail' : 'recordings.retryError', { error: rerunDetail })}
        </p>
      )}
      {rec.status !== 'processing' && rec.transcript?.has_timestamps && (
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <label htmlFor="speaker-count" className="font-semibold">
            {t('speakers.count')}
          </label>
          <select
            id="speaker-count"
            value={requested ?? ''}
            disabled={speakerCount.isPending}
            onChange={(e) => speakerCount.mutate(e.target.value === '' ? null : Number(e.target.value))}
            className="min-h-11 rounded-md border border-border bg-card px-2 py-1.5"
          >
            <option value="">{t('speakers.automatic')}</option>
            {Array.from({ length: Math.max(MAX_SPEAKER_CHOICE, requested ?? 0) }, (_, i) => (
              <option key={i + 1} value={i + 1}>
                {i + 1}
              </option>
            ))}
          </select>
          <span className="text-muted-foreground">{t('speakers.countHelp')}</span>
          {speakerCount.isError && (
            <span role="alert" className="text-destructive">
              {t(countRefused ? 'speakers.countRefused' : 'speakers.countError')}
            </span>
          )}
        </div>
      )}
      {rec.status === 'processing' && (
        <p role="status" className="flex items-center gap-2 rounded-lg border border-primary/40 bg-primary/10 p-4 text-sm text-primary">
          <span aria-hidden="true" className="motion-safe:animate-pulse">
            ◌
          </span>
          {t('recordings.processing')}
        </p>
      )}

      {rec.no_speech && (
        <p role="status" className="rounded-lg border border-dashed border-border p-4 text-sm text-muted-foreground">
          {t('recordings.noSpeechDetail')}
        </p>
      )}

      {/* Sticky under the header, so the transcript and the action times stay playable. */}
      <div className="sticky top-14 z-10 -mx-1 rounded-full bg-background/90 p-1 backdrop-blur">
        <audio ref={audio} controls src={`/api/recordings/${id}/audio`} className="block w-full" />
      </div>

      {rec.summary && (
        <>
          <Pane
            title={t('summaries.summary')}
            actions={<CopyButton text={rec.summary.summary_md} label={t('summaries.copy')} />}
          >
            {/* No raw HTML (react-markdown default) and no images: a remote URL in LLM output would leak. */}
            <div className="space-y-2 leading-relaxed [&_a]:text-primary [&_a]:underline [&_h1]:mt-4 [&_h1]:font-semibold [&_h1]:text-primary [&_h1:first-child]:mt-0 [&_h2]:mt-4 [&_h2]:font-semibold [&_h2]:text-primary [&_h2:first-child]:mt-0 [&_h3]:mt-3 [&_h3]:font-semibold [&_li]:ml-5 [&_li::marker]:text-muted-foreground [&_ol]:list-decimal [&_ul]:list-disc">
              <Markdown disallowedElements={['img']}>{rec.summary.summary_md}</Markdown>
            </div>
          </Pane>
          <Pane title={t('summaries.mindmap')}>
            <Mindmap code={rec.summary.mindmap} />
          </Pane>
          <Pane title={t('summaries.actions.title')}>
            <ActionsTable recordingId={id} actions={rec.summary.actions} onSeek={seek} />
          </Pane>
        </>
      )}

      {rec.transcript && (
        <Pane
          title={t('transcription.title')}
          actions={
            <>
              <CopyButton text={transcriptCopyText(rec.transcript, speakerName)} label={t('transcription.copy')} />
              <button
                type="button"
                aria-label={t('transcription.download')}
                onClick={() =>
                  downloadText(
                    transcriptFileText(rec.transcript!, speakerName),
                    transcriptFileName(rec.summary?.title ?? rec.title, rec.id),
                  )
                }
                className="rounded-md border border-border bg-card px-3 py-1 text-sm font-semibold hover:text-primary"
              >
                {t('transcription.downloadShort')}
              </button>
            </>
          }
        >
          <TranscriptView
            transcript={rec.transcript}
            speakerName={speakerName}
            onRename={onRename}
            audio={audio}
            onSeek={seek}
          />
        </Pane>
      )}
    </div>
  )
}
