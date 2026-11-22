import { MicIcon, SearchXIcon } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia } from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { Item, ItemActions, ItemContent, ItemDescription, ItemGroup, ItemTitle } from '@/components/ui/item'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import { fold } from '@/shared/text'
import { formatDate, formatTimestamp } from '@/shared/time'
import { useRecordings } from './api'
import { StatusBadge } from './StatusBadge'
import { recordingTitle } from './title'

export function RecordingsPage() {
  const { t } = useTranslation()
  const { data, isLoading, isError } = useRecordings()
  const [search, setSearch] = useState('')
  // Compared without case or accents ("reunion" finds "Réunion"), on the title as the list shows it.
  const query = fold(search.trim())
  const matches = query ? data?.filter((r) => fold(recordingTitle(r, t)).includes(query)) : data

  return (
    <div className="mx-auto w-full max-w-5xl px-4 py-6 sm:py-8">
      <h1 className="mb-4 flex items-baseline gap-2 text-2xl font-semibold tracking-tight">
        {t('recordings.title')}
        {data && data.length > 0 && (
          <span className="text-base font-normal text-muted-foreground tabular-nums">
            {query ? `${matches?.length ?? 0}/${data.length}` : data.length}
          </span>
        )}
      </h1>
      {isLoading && (
        <div role="status" aria-label={t('common.loading')} className="space-y-2">
          {[0, 1, 2, 3].map((i) => (
            <Skeleton key={i} className="h-16" />
          ))}
        </div>
      )}
      {isError && (
        <Alert variant="destructive">
          <AlertDescription>{t('recordings.loadError')}</AlertDescription>
        </Alert>
      )}
      {data?.length === 0 && (
        <Empty className="border">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <MicIcon aria-hidden="true" />
            </EmptyMedia>
            <EmptyDescription>{t('recordings.empty')}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}
      {data && data.length > 0 && (
        <div className="mb-4 max-w-sm space-y-2">
          <Label htmlFor="recordings-search">{t('recordings.searchByTitle')}</Label>
          <Input
            id="recordings-search"
            type="search"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            className="max-md:h-11"
          />
        </div>
      )}
      {query && data && data.length > 0 && matches?.length === 0 && (
        <Empty role="status" className="border">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <SearchXIcon aria-hidden="true" />
            </EmptyMedia>
            <EmptyDescription>{t('recordings.noMatches')}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}
      {matches && matches.length > 0 && (
        <ItemGroup className="gap-2">
          {matches.map((r) => (
            // ItemGroup is role="list": each row is wrapped so the link keeps its own role.
            <div key={r.id} role="listitem">
              <Item variant="outline" size="sm" render={<Link to={`/recordings/${r.id}`} />}>
                <ItemContent className="min-w-0">
                  <ItemTitle className="block max-w-full truncate">{recordingTitle(r, t)}</ItemTitle>
                  <ItemDescription className="tabular-nums">
                    <time dateTime={r.created_at}>{formatDate(r.created_at)}</time>
                    {' · '}
                    {r.duration_s != null ? formatTimestamp(r.duration_s) : '—'}
                  </ItemDescription>
                </ItemContent>
                <ItemActions className="shrink-0">
                  <StatusBadge recording={r} />
                </ItemActions>
              </Item>
            </div>
          ))}
        </ItemGroup>
      )}
    </div>
  )
}
