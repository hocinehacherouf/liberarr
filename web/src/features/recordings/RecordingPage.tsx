import { ArrowLeftIcon, CircleCheckIcon, CircleDashedIcon, CircleXIcon, DownloadIcon, LoaderCircleIcon, MicOffIcon, TriangleAlertIcon } from 'lucide-react'
import { useCallback, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import Markdown from 'react-markdown'
import { Link, useParams } from 'react-router'
import { toast } from 'sonner'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardAction, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { recordingTitle } from '@/features/recordings/title'
import { ApiError } from '@/shared/api'
import { CopyButton } from '@/shared/CopyButton'
import { formatDate, formatTimestamp } from '@/shared/time'
import { ActionsTable } from '@/features/summaries/ActionsTable'
import { Mindmap } from '@/features/summaries/Mindmap'
import { TranscriptView } from '@/features/transcription/TranscriptView'
import { downloadText, transcriptCopyText, transcriptFileName, transcriptFileText } from '@/features/transcription/text'
import { useRecording, useRenameSpeaker, useRerun, useSpeakerCount, type Job } from '@/features/recordings/api'
import { AudioBar } from '@/features/recordings/AudioBar'
import { RecordingMenu } from '@/features/recordings/RecordingMenu'
import { StatusBadge } from '@/features/recordings/StatusBadge'

// An icon per pipeline step state: the shape tells the state apart without the color.
const JOB: Record<Job['status'], { Icon: typeof CircleCheckIcon; className: string }> = {
  done: { Icon: CircleCheckIcon, className: 'text-success' },
  failed: { Icon: CircleXIcon, className: 'text-destructive' },
  running: { Icon: LoaderCircleIcon, className: 'text-primary motion-safe:animate-spin' },
  queued: { Icon: CircleDashedIcon, className: 'text-muted-foreground' },
}
const MAX_SPEAKER_CHOICE = 10
const AUTOMATIC = 'auto'

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
  const [tab, setTab] = useState<string | null>(null)

  const seek = (seconds: number) => {
    if (!audio.current) return
    audio.current.currentTime = seconds
    void audio.current.play().catch(() => {})
  }

  const back = (
    <Button variant="ghost" size="sm" nativeButton={false} render={<Link to="/" />} className="-ml-2.5 max-md:h-11">
      <ArrowLeftIcon data-icon="inline-start" aria-hidden="true" />
      {t('recordings.back')}
    </Button>
  )
  if (isLoading)
    return (
      <div role="status" aria-label={t('common.loading')} className="mx-auto w-full max-w-5xl space-y-4 px-4 py-6 sm:py-8">
        <Skeleton className="h-8 w-24" />
        <Skeleton className="h-8 w-2/3" />
        <Skeleton className="h-5 w-1/2" />
        <Skeleton className="h-9 w-72 max-w-full" />
        <Skeleton className="h-64" />
      </div>
    )
  if (isError || !rec)
    return (
      <div className="mx-auto w-full max-w-5xl space-y-4 px-4 py-6 sm:py-8">
        {back}
        <Alert variant="destructive">
          <AlertDescription>{t('recordings.notFound')}</AlertDescription>
        </Alert>
      </div>
    )

  const processing = rec.status === 'processing'
  const failedStep = rec.status === 'failed' ? rec.failed_step : null
  const failedJob = failedStep ? rec.jobs.findLast((j) => j.step === failedStep) : undefined
  const requested = rec.speakers.requested
  const speakerName = (n: number) => rec.speakers.names[n] ?? t('speakers.label', { n: n + 1 })
  // The latest job of each step, in the order the steps first ran.
  const steps = [...new Set(rec.jobs.map((j) => j.step))].map((step) => rec.jobs.findLast((j) => j.step === step)!)
  const title = rec.summary?.title ?? recordingTitle(rec, t)
  const speakerChoices = [
    AUTOMATIC,
    ...Array.from({ length: Math.max(MAX_SPEAKER_CHOICE, requested ?? 0) }, (_, i) => String(i + 1)),
  ]

  const rerunStep = (step: string) =>
    rerun.mutate(step, {
      onError: (e) => {
        const detail = e instanceof ApiError ? e.message : ''
        // The server refuses a summary rerun (409) when `liberarr init` chose no engine.
        const noEngine = e instanceof ApiError && e.status === 409 && step === 'summarize'
        toast.error(
          noEngine
            ? t('recordings.noSummaryEngine')
            : t(detail ? 'recordings.retryErrorWithDetail' : 'recordings.retryError', { error: detail }),
        )
      },
    })
  const setSpeakerCount = (value: string | null) =>
    speakerCount.mutate(value == null || value === AUTOMATIC ? null : Number(value), {
      // 409: a step is running, or speaker detection is off.
      onError: (e) => toast.error(t(e instanceof ApiError && e.status === 409 ? 'speakers.countRefused' : 'speakers.countError')),
    })

  // An empty transcript (a recording with no speech) gets no tab and no speaker-count control.
  const transcript = rec.transcript && (rec.transcript.segments.length > 0 || rec.transcript.text.trim() !== '') ? rec.transcript : null
  // A tab exists only when its data does; the first one is the default.
  const tabs = [
    rec.summary && 'summary',
    transcript && 'transcript',
    rec.summary && 'actions',
    rec.summary && 'mindmap',
  ].filter((v): v is string => !!v)
  const active = tab && tabs.includes(tab) ? tab : tabs[0]

  return (
    <div className="flex flex-1 flex-col">
      <div className="mx-auto w-full max-w-5xl flex-1 space-y-4 px-4 pt-6 pb-8 sm:pt-8">
        <div className="space-y-2">
          {back}
          <div className="flex items-start justify-between gap-2">
            <h1 className="min-w-0 text-2xl font-semibold tracking-tight text-balance wrap-break-word">{title}</h1>
            {!processing && <RecordingMenu disabled={rerun.isPending} onRerun={rerunStep} />}
          </div>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-2 text-sm text-muted-foreground">
            <StatusBadge recording={rec} />
            <time dateTime={rec.created_at}>{formatDate(rec.created_at)}</time>
            {rec.duration_s != null && <span className="tabular-nums">{formatTimestamp(rec.duration_s)}</span>}
            {steps.length > 0 && (
              <ol aria-label={t('recordings.pipeline')} className="flex flex-wrap items-center gap-1.5">
                {steps.map((j) => {
                  const { Icon, className } = JOB[j.status]
                  return (
                    <li key={j.step}>
                      <Badge variant="outline">
                        <Icon aria-hidden="true" className={className} />
                        {t(`recordings.steps.${j.step}`, { defaultValue: j.step })}
                        <span className="sr-only">{t(`recordings.jobStatus.${j.status}`)}</span>
                      </Badge>
                    </li>
                  )
                })}
              </ol>
            )}
          </div>
        </div>

        {failedStep && (
          <Alert variant="destructive">
            <TriangleAlertIcon aria-hidden="true" />
            <AlertTitle>
              {t(failedJob?.error ? 'recordings.stepFailedWithError' : 'recordings.stepFailed', {
                step: t(`recordings.steps.${failedStep}`, { defaultValue: failedStep }),
                error: failedJob?.error,
              })}
            </AlertTitle>
            <div className="col-start-2 mt-2">
              <Button variant="outline" size="sm" disabled={rerun.isPending} onClick={() => rerunStep(failedStep)} className="max-md:h-11">
                {t('recordings.retry')}
              </Button>
            </div>
          </Alert>
        )}
        {processing && (
          <Alert role="status">
            {/* The alert is the status: the spinner is decoration. */}
            <Spinner aria-hidden="true" role="presentation" className="animate-none motion-safe:animate-spin" />
            <AlertTitle>{t('recordings.processing')}</AlertTitle>
          </Alert>
        )}
        {rec.no_speech && (
          <Alert role="status">
            <MicOffIcon aria-hidden="true" />
            <AlertDescription>{t('recordings.noSpeechDetail')}</AlertDescription>
          </Alert>
        )}

        {active && (
          <Tabs value={active} onValueChange={setTab}>
            <TabsList className="max-w-full justify-start overflow-x-auto group-data-horizontal/tabs:max-md:h-11!">
              {tabs.map((value) => (
                <TabsTrigger key={value} value={value}>
                  {t(
                    {
                      summary: 'summaries.summary',
                      transcript: 'transcription.title',
                      actions: 'summaries.actions.title',
                      mindmap: 'summaries.mindmap',
                    }[value]!,
                  )}
                </TabsTrigger>
              ))}
            </TabsList>
            {rec.summary && (
              <>
                <TabsContent value="summary">
                  <Card>
                    <CardHeader>
                      <CardTitle>{t('summaries.summary')}</CardTitle>
                      <CardAction>
                        <CopyButton text={rec.summary.summary_md} label={t('summaries.copy')} />
                      </CardAction>
                    </CardHeader>
                    <CardContent>
                      {/* No raw HTML (react-markdown default) and no images: a remote URL in LLM output would leak. */}
                      <div className="space-y-2 text-base leading-relaxed [&_a]:text-primary [&_a]:underline [&_h1]:mt-4 [&_h1]:font-semibold [&_h1]:text-primary [&_h1:first-child]:mt-0 [&_h2]:mt-4 [&_h2]:font-semibold [&_h2]:text-primary [&_h2:first-child]:mt-0 [&_h3]:mt-3 [&_h3]:font-semibold [&_li]:ml-5 [&_li::marker]:text-muted-foreground [&_ol]:list-decimal [&_ul]:list-disc">
                        <Markdown disallowedElements={['img']}>{rec.summary.summary_md}</Markdown>
                      </div>
                    </CardContent>
                  </Card>
                </TabsContent>
                <TabsContent value="actions">
                  <Card>
                    <CardContent>
                      <ActionsTable recordingId={id} actions={rec.summary.actions} onSeek={seek} />
                    </CardContent>
                  </Card>
                </TabsContent>
                <TabsContent value="mindmap">
                  <Card>
                    <CardContent>
                      <Mindmap code={rec.summary.mindmap} />
                    </CardContent>
                  </Card>
                </TabsContent>
              </>
            )}
            {transcript && (
              <TabsContent value="transcript">
                <Card>
                  <CardContent className="space-y-4">
                    <div className="flex flex-wrap items-center gap-2">
                      {!processing && transcript.has_timestamps && (
                        <>
                          <Label htmlFor="speaker-count">{t('speakers.count')}</Label>
                          <Select
                            value={requested == null ? AUTOMATIC : String(requested)}
                            onValueChange={setSpeakerCount}
                            disabled={speakerCount.isPending}
                          >
                            <SelectTrigger id="speaker-count" className="data-[size=default]:max-md:h-11">
                              <SelectValue>
                                {(value: string) => (value === AUTOMATIC ? t('speakers.automatic') : value)}
                              </SelectValue>
                            </SelectTrigger>
                            <SelectContent>
                              {speakerChoices.map((value) => (
                                <SelectItem key={value} value={value}>
                                  {value === AUTOMATIC ? t('speakers.automatic') : value}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                          <span className="text-sm text-muted-foreground">{t('speakers.countHelp')}</span>
                        </>
                      )}
                      <div className="ml-auto flex items-center gap-2">
                        <CopyButton text={transcriptCopyText(transcript, speakerName)} label={t('transcription.copy')} />
                        <Button
                          variant="outline"
                          size="sm"
                          aria-label={t('transcription.download')}
                          className="max-md:h-11"
                          onClick={() =>
                            downloadText(
                              transcriptFileText(transcript, speakerName),
                              transcriptFileName(rec.summary?.title ?? rec.title, rec.id),
                            )
                          }
                        >
                          <DownloadIcon data-icon="inline-start" aria-hidden="true" />
                          {t('transcription.downloadShort')}
                        </Button>
                      </div>
                    </div>
                    <TranscriptView
                      transcript={transcript}
                      speakerName={speakerName}
                      onRename={onRename}
                      audio={audio}
                      onSeek={seek}
                    />
                  </CardContent>
                </Card>
              </TabsContent>
            )}
          </Tabs>
        )}
      </div>
      <AudioBar audio={audio} src={`/api/recordings/${id}/audio`} durationHint={rec.duration_s} />
    </div>
  )
}
