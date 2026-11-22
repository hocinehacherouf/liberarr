import { ChevronsUpDownIcon, UserIcon } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link, NavLink, useLocation } from 'react-router'
import { useLogout, useMe } from '@/features/auth/api'
import { useRecordings } from '@/features/recordings/api'
import { recordingStatus } from '@/features/recordings/StatusBadge'
import { recordingTitle } from '@/features/recordings/title'
import { Logo } from '@/shared/Logo'
import { fold } from '@/shared/text'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInput,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSkeleton,
  SidebarRail,
  useSidebar,
} from '@/components/ui/sidebar'
import { ThemeMenu } from './ThemeMenu'

function RecordingsMenu({ search }: { search: string }) {
  const { t } = useTranslation()
  const { data, isLoading, isError } = useRecordings()
  const { pathname } = useLocation()
  const { setOpenMobile } = useSidebar()

  if (isLoading) {
    return (
      <SidebarMenu>
        {[0, 1, 2].map((i) => (
          <SidebarMenuItem key={i}>
            <SidebarMenuSkeleton />
          </SidebarMenuItem>
        ))}
      </SidebarMenu>
    )
  }
  if (isError && !data) {
    return (
      <p role="alert" className="px-3 py-2 text-sm text-destructive">
        {t('recordings.loadError')}
      </p>
    )
  }
  if (data?.length === 0) return <p className="px-3 py-2 text-sm text-muted-foreground">{t('recordings.empty')}</p>

  const query = fold(search.trim())
  const matches = data?.filter((r) => fold(recordingTitle(r, t)).includes(query)) ?? []
  if (matches.length === 0 && query) {
    return (
      <p role="status" className="px-3 py-2 text-sm text-muted-foreground">
        {t('recordings.noMatches')}
      </p>
    )
  }

  return (
    <SidebarMenu>
      {matches.map((r) => {
        const { Icon, iconClass, label } = recordingStatus(r)
        const to = `/recordings/${r.id}`
        return (
          <SidebarMenuItem key={r.id}>
            <SidebarMenuButton
              isActive={pathname === to}
              className="max-md:h-11"
              render={<NavLink to={to} onClick={() => setOpenMobile(false)} />}
            >
              <Icon aria-hidden="true" className={iconClass} />
              <span className="truncate">{recordingTitle(r, t)}</span>
              <span className="sr-only">{t(label)}</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        )
      })}
    </SidebarMenu>
  )
}

export function AppSidebar() {
  const { t } = useTranslation()
  const me = useMe()
  const logout = useLogout()
  const [search, setSearch] = useState('')

  return (
    <Sidebar collapsible="icon">
      <SidebarHeader>
        <Link
          to="/"
          className="flex items-center overflow-hidden rounded-xl px-2 py-1.5 text-lg outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring group-data-[collapsible=icon]:px-0.5"
        >
          <Logo />
        </Link>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup className="group-data-[collapsible=icon]:hidden">
          <SidebarGroupLabel>{t('recordings.title')}</SidebarGroupLabel>
          <SidebarGroupContent className="space-y-2">
            <SidebarInput
              type="search"
              aria-label={t('recordings.searchByTitle')}
              placeholder={t('common.search')}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              className="max-md:h-11"
            />
            <RecordingsMenu search={search} />
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
      <SidebarFooter>
        <SidebarMenu>
          <SidebarMenuItem>
            <DropdownMenu>
              <DropdownMenuTrigger render={<SidebarMenuButton className="max-md:h-11" />}>
                <UserIcon />
                <span className="truncate">{me.data?.username}</span>
                <ChevronsUpDownIcon className="ml-auto" />
              </DropdownMenuTrigger>
              <DropdownMenuContent side="top" align="start">
                <DropdownMenuSub>
                  <DropdownMenuSubTrigger className="max-md:min-h-11">{t('theme.title')}</DropdownMenuSubTrigger>
                  <DropdownMenuSubContent>
                    <ThemeMenu />
                  </DropdownMenuSubContent>
                </DropdownMenuSub>
                <DropdownMenuSeparator />
                <DropdownMenuItem className="max-md:min-h-11" onClick={() => logout.mutate()}>
                  {t('auth.signOut')}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  )
}
