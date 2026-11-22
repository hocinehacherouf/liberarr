import { LogOutIcon } from 'lucide-react'
import { useEffect } from 'react'
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
import { useRecordings } from '@/features/recordings/api'
import { recordingStatus } from '@/features/recordings/StatusBadge'
import { recordingTitle } from '@/features/recordings/title'
import { fold } from '@/shared/text'

// cmdk's own matching is accent-sensitive: compare folded text, as the sidebar search does.
// A recording's value starts with its id (see below): drop it, or typing a digit would match ids.
const filter = (value: string, search: string) =>
  fold(value.replace(/^\d+ /, '')).includes(fold(search.trim())) ? 1 : 0

/** Cmd/Ctrl+K: jump to a recording, switch the theme, sign out. */
export function CommandPalette({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const { setTheme } = useTheme()
  const logout = useLogout()
  const { data } = useRecordings()

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() === 'k' && (event.metaKey || event.ctrlKey)) {
        event.preventDefault()
        onOpenChange(!open)
      }
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [open, onOpenChange])

  const run = (action: () => void) => () => {
    onOpenChange(false)
    action()
  }

  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      title={t('palette.title')}
      description={t('palette.description')}
    >
      <Command filter={filter}>
        <CommandInput placeholder={t('common.search')} />
        <CommandList>
          <CommandEmpty>{t('recordings.noMatches')}</CommandEmpty>
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
