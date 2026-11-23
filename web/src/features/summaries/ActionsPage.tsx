import { ListChecksIcon } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link, useNavigate } from 'react-router'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia } from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { formatDate } from '@/shared/time'
import { useActions, type Action } from '@/features/recordings/api'
import { recordingTitle } from '@/features/recordings/title'
import { ActionsTable } from '@/features/summaries/ActionsTable'

type Filter = 'open' | 'all'

// The action's text is part of the key: after a reload a different action can sit at the same index.
const rowKey = (recordingId: number, index: number, a: Action) => `${recordingId}:${index}:${a.action}`

export function ActionsPage() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const { data, isLoading, isError } = useActions()
  const [filter, setFilter] = useState<Filter>('open')
  // Rows ticked here stay visible under Open until the filter changes, so a mis-click can be undone.
  const [kept, setKept] = useState<Set<string>>(new Set())

  const isVisible = (recordingId: number) => (a: Action, index: number) =>
    filter === 'all' || !a.done || kept.has(rowKey(recordingId, index, a))
  const groups = data?.filter((g) => g.actions.some(isVisible(g.recording.id)))

  const body = (
    <>
      {isLoading && (
        <div role="status" aria-label={t('common.loading')} className="space-y-2">
          {[0, 1, 2].map((i) => (
            <Skeleton key={i} className="h-24" />
          ))}
        </div>
      )}
      {isError && !data && (
        <Alert variant="destructive">
          <AlertDescription>{t('actions.loadError')}</AlertDescription>
        </Alert>
      )}
      {groups?.length === 0 && (
        <Empty className="border">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <ListChecksIcon aria-hidden="true" />
            </EmptyMedia>
            <EmptyDescription>{t(filter === 'open' ? 'actions.noneOpen' : 'actions.none')}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}
      {groups?.map(({ recording, actions }) => {
        const headingId = `actions-${recording.id}`
        return (
          <section key={recording.id} className="mb-6">
            <h2 id={headingId} className="mb-2 flex flex-wrap items-baseline gap-x-3 text-lg font-medium">
              <Link to={`/recordings/${recording.id}`} className="hover:underline">
                {recordingTitle(recording, t)}
              </Link>
              <time dateTime={recording.created_at} className="text-sm font-normal text-muted-foreground tabular-nums">
                {formatDate(recording.created_at)}
              </time>
            </h2>
            <ActionsTable
              recordingId={recording.id}
              actions={actions}
              labelledBy={headingId}
              visible={isVisible(recording.id)}
              onToggle={(index) => setKept((k) => new Set(k).add(rowKey(recording.id, index, actions[index])))}
              // Rounded up like the palette's hit link: the audio never starts before the words.
              onSeek={(s) => void navigate(`/recordings/${recording.id}?t=${Math.ceil(s * 100) / 100}`)}
            />
          </section>
        )
      })}
    </>
  )

  return (
    <div className="mx-auto w-full max-w-5xl px-4 py-6 sm:py-8">
      <h1 className="mb-4 text-2xl font-semibold tracking-tight">{t('actions.title')}</h1>
      <Tabs
        value={filter}
        onValueChange={(value) => {
          setFilter(value as Filter)
          setKept(new Set())
        }}
      >
        <TabsList aria-label={t('actions.filter')} className="mb-2">
          <TabsTrigger value="open">{t('actions.open')}</TabsTrigger>
          <TabsTrigger value="all">{t('actions.all')}</TabsTrigger>
        </TabsList>
        {/* One panel, for the selected filter: the body is rendered once. */}
        <TabsContent value={filter}>{body}</TabsContent>
      </Tabs>
    </div>
  )
}
