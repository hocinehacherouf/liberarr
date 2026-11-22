import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router'
import { Loading } from '../../shared/Loading'
import { fold } from '../../shared/text'
import { formatDate, formatTimestamp } from '../../shared/time'
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
      {isLoading && <Loading />}
      {isError && (
        <p role="alert" className="rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm text-destructive">
          {t('recordings.loadError')}
        </p>
      )}
      {data?.length === 0 && (
        <p className="rounded-xl border border-dashed border-border bg-card p-8 text-center text-muted-foreground">
          {t('recordings.empty')}
        </p>
      )}
      {data && data.length > 0 && (
        <label className="mb-4 block max-w-sm text-sm font-medium text-muted-foreground">
          {t('recordings.searchByTitle')}
          <input
            type="search"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            className="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-foreground focus:border-primary"
          />
        </label>
      )}
      {query && data && data.length > 0 && matches?.length === 0 && (
        <p role="status" className="rounded-xl border border-dashed border-border bg-card p-8 text-center text-muted-foreground">
          {t('recordings.noMatches')}
        </p>
      )}
      {matches && matches.length > 0 && (
        <ul className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
          {matches.map((r) => (
            <li key={r.id}>
              <Link
                to={`/recordings/${r.id}`}
                className="grid grid-cols-[1fr_auto] items-center gap-x-4 gap-y-1 px-4 py-3 hover:bg-primary/5 focus-visible:-outline-offset-2 sm:grid-cols-[1fr_auto_auto_auto]"
              >
                <span className="col-span-2 truncate font-medium sm:col-span-1">
                  {recordingTitle(r, t)}
                </span>
                <span className="flex gap-3 text-sm text-muted-foreground sm:contents">
                  <time dateTime={r.created_at}>{formatDate(r.created_at)}</time>
                  <span className="font-mono tabular-nums sm:w-14 sm:text-right">
                    {r.duration_s != null ? formatTimestamp(r.duration_s) : '—'}
                  </span>
                </span>
                <span className="justify-self-end">
                  <StatusBadge recording={r} />
                </span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
