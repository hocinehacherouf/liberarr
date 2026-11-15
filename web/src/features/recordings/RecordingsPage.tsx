import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router'
import { Loading } from '../../shared/Loading'
import { formatDate, formatTimestamp } from '../../shared/time'
import { useRecordings } from './api'
import { StatusBadge } from './StatusBadge'

export function RecordingsPage() {
  const { t } = useTranslation()
  const { data, isLoading, isError } = useRecordings()
  const [search, setSearch] = useState('')
  // Compared without case or accents ("reunion" finds "Réunion"), on the title as the list shows it.
  const fold = (s: string) => s.normalize('NFD').replace(/\p{Diacritic}/gu, '').toLowerCase()
  const query = fold(search.trim())
  const matches = query
    ? data?.filter((r) => fold(r.title ?? t('recordings.untitled', { id: r.id })).includes(query))
    : data

  return (
    <main className="mx-auto max-w-5xl px-4 py-6 sm:py-8">
      <h1 className="mb-4 flex items-baseline gap-2 text-2xl font-semibold tracking-tight">
        {t('recordings.title')}
        {data && data.length > 0 && (
          <span className="text-base font-normal text-muted tabular-nums">
            {query ? `${matches?.length ?? 0}/${data.length}` : data.length}
          </span>
        )}
      </h1>
      {isLoading && <Loading />}
      {isError && (
        <p role="alert" className="rounded-lg border border-err/40 bg-err/10 p-4 text-sm text-err">
          {t('recordings.loadError')}
        </p>
      )}
      {data?.length === 0 && (
        <p className="rounded-xl border border-dashed border-line bg-surface p-8 text-center text-muted">
          {t('recordings.empty')}
        </p>
      )}
      {data && data.length > 0 && (
        <label className="mb-4 block max-w-sm text-sm font-medium text-muted">
          {t('recordings.searchByTitle')}
          <input
            type="search"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            className="mt-1 w-full rounded-md border border-line bg-canvas px-3 py-2 text-fg focus:border-accent"
          />
        </label>
      )}
      {query && data && data.length > 0 && matches?.length === 0 && (
        <p role="status" className="rounded-xl border border-dashed border-line bg-surface p-8 text-center text-muted">
          {t('recordings.noMatches')}
        </p>
      )}
      {matches && matches.length > 0 && (
        <ul className="divide-y divide-line overflow-hidden rounded-xl border border-line bg-surface">
          {matches.map((r) => (
            <li key={r.id}>
              <Link
                to={`/recordings/${r.id}`}
                className="grid grid-cols-[1fr_auto] items-center gap-x-4 gap-y-1 px-4 py-3 hover:bg-accent/5 focus-visible:-outline-offset-2 sm:grid-cols-[1fr_auto_auto_auto]"
              >
                <span className="col-span-2 truncate font-medium sm:col-span-1">
                  {r.title ?? t('recordings.untitled', { id: r.id })}
                </span>
                <span className="flex gap-3 text-sm text-muted sm:contents">
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
    </main>
  )
}
