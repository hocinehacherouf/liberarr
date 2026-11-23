import { ListChecksIcon, LogOutIcon, TextSearchIcon } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useNavigate } from 'react-router'
import { THEMES } from '@/app/themes'
import { useTheme } from '@/components/theme-provider'
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import { useLogout } from '@/features/auth/api'
import { SEARCH_HITS, searchQuery, useRecordings, useTranscriptSearch } from '@/features/recordings/api'
import { recordingStatus } from '@/features/recordings/StatusBadge'
import { recordingTitle } from '@/features/recordings/title'
import { formatTimestamp } from '@/shared/time'
import { fold } from '@/shared/text'

const DEBOUNCE_MS = 250

// cmdk's own matching is accent-sensitive: compare folded text, as the sidebar search does.
// A recording's value starts with its id (see below): drop it, or typing a digit would match ids.
// A transcript hit is already matched by the server: let it through.
const filter = (value: string, search: string) =>
  value.startsWith('hit:') ? 1 : fold(value.replace(/^\d+ /, '')).includes(fold(search.trim())) ? 1 : 0

/** Cmd/Ctrl+K: go to the actions or a recording, switch the theme, sign out. */
export function CommandPalette({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const { setTheme } = useTheme()
  const logout = useLogout()
  const { data } = useRecordings()
  const [input, setInput] = useState('')
  const q = searchQuery(input)
  const [debounced, setDebounced] = useState<string | null>(null)
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(q), DEBOUNCE_MS)
    return () => clearTimeout(timer)
  }, [q])
  const search = useTranscriptSearch(debounced)
  // A hit of an older query is never shown, nor selectable.
  const fresh = q !== null && debounced === q
  const hits = fresh ? search.data?.hits : undefined

  const setOpen = useCallback(
    (next: boolean) => {
      if (!next) {
        setInput('')
        setDebounced(null)
      }
      onOpenChange(next)
    },
    [onOpenChange],
  )

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() === 'k' && (event.metaKey || event.ctrlKey)) {
        event.preventDefault()
        setOpen(!open)
      }
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [open, setOpen])

  const run = (action: () => void) => () => {
    setOpen(false)
    action()
  }

  return (
    <CommandDialog
      open={open}
      onOpenChange={setOpen}
      title={t('palette.title')}
      description={t('palette.description')}
    >
      <Command filter={filter}>
        <CommandInput placeholder={t('common.search')} value={input} onValueChange={setInput} />
        <CommandList>
          {(q === null || (fresh && !search.isFetching && !search.isError)) && <CommandEmpty>{t('recordings.noMatches')}</CommandEmpty>}
          {data && data.length > 0 && (
            <CommandGroup heading={t('recordings.title')}>
              {data.map((r) => {
                const { Icon } = recordingStatus(r)
                const title = recordingTitle(r, t)
                return (
                  // The id keeps two recordings with the same title apart.
                  <CommandItem
                    key={r.id}
                    value={`${r.id} ${fold(title)}`}
                    onSelect={run(() => void navigate(`/recordings/${r.id}`))}
                  >
                    <Icon aria-hidden="true" />
                    <span className="truncate">{title}</span>
                  </CommandItem>
                )
              })}
            </CommandGroup>
          )}
          {hits && hits.length > 0 && (
            <CommandGroup heading={t('palette.transcripts')}>
              {hits.map((h, i) => {
                const rec = data?.find((r) => r.id === h.recording_id)
                if (!rec) return null
                return (
                  <CommandItem
                    key={`${h.recording_id}:${i}`}
                    value={`hit:${h.recording_id}:${i}`}
                    onSelect={run(() => void navigate(`/recordings/${h.recording_id}?t=${h.start === null ? 0 : Math.ceil(h.start * 100) / 100}`))}
                    className="items-start"
                  >
                    <TextSearchIcon aria-hidden="true" className="mt-0.5" />
                    <div className="min-w-0 flex-1">
                      <div className="flex items-baseline gap-2">
                        <span className="truncate">{recordingTitle(rec, t)}</span>
                        {h.start !== null && (
                          <span className="font-mono tabular-nums text-muted-foreground">{formatTimestamp(h.start)}</span>
                        )}
                      </div>
                      <p className="line-clamp-2 font-normal text-muted-foreground">
                        {h.before}
                        <mark className="rounded-sm bg-primary/15 text-foreground">{h.match}</mark>
                        {h.after}
                      </p>
                    </div>
                  </CommandItem>
                )
              })}
              {search.data?.truncated && (
                <p className="px-3 py-2 text-xs text-muted-foreground">{t('palette.moreHits', { count: SEARCH_HITS })}</p>
              )}
            </CommandGroup>
          )}
          {q !== null && (!fresh || (search.isFetching && !search.data)) && (
            <p role="status" className="px-3 py-2 text-sm text-muted-foreground">
              {t('palette.searching')}
            </p>
          )}
          {fresh && search.isError && (
            <p role="status" className="px-3 py-2 text-sm text-muted-foreground">
              {t('palette.searchError')}
            </p>
          )}
          <CommandGroup>
            <CommandItem value={t('palette.goToActions')} onSelect={run(() => void navigate('/actions'))}>
              <ListChecksIcon aria-hidden="true" />
              {t('palette.goToActions')}
            </CommandItem>
          </CommandGroup>
          <CommandGroup heading={t('theme.title')}>
            {THEMES.map(([name, Icon]) => (
              <CommandItem key={name} value={t(`theme.${name}`)} onSelect={run(() => setTheme(name))}>
                <Icon />
                {t(`theme.${name}`)}
              </CommandItem>
            ))}
          </CommandGroup>
          <CommandGroup>
            <CommandItem value={t('auth.signOut')} onSelect={run(() => logout.mutate())}>
              <LogOutIcon />
              {t('auth.signOut')}
            </CommandItem>
          </CommandGroup>
        </CommandList>
      </Command>
    </CommandDialog>
  )
}
