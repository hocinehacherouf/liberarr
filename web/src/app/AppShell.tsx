import { SearchIcon } from 'lucide-react'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Outlet } from 'react-router'
import { Button } from '@/components/ui/button'
import { Kbd } from '@/components/ui/kbd'
import { Separator } from '@/components/ui/separator'
import { SidebarInset, SidebarProvider, SidebarTrigger } from '@/components/ui/sidebar'
import { AppSidebar } from '@/app/AppSidebar'
import { CommandPalette } from '@/app/CommandPalette'
import { useProcessing } from '@/features/recordings/api'

const SHORTCUT = /Mac|iPhone|iPad/.test(navigator.userAgent) ? '⌘K' : 'Ctrl K'

/** The sidebar, the top bar and the page of every route but login. */
export function AppShell() {
  const { t } = useTranslation()
  const [paletteOpen, setPaletteOpen] = useState(false)

  // The one place that refetches the actions when processing ends (a summary may have landed after the last poll):
  // the shell is mounted on every route, phone or desktop; the sidebar sheet and the page are not.
  const qc = useQueryClient()
  const processing = useProcessing()
  const wasProcessing = useRef(false)
  useEffect(() => {
    if (wasProcessing.current && !processing) void qc.invalidateQueries({ queryKey: ['recordings', 'actions'] })
    wasProcessing.current = processing
  }, [processing, qc])

  return (
    <SidebarProvider>
      <AppSidebar />
      {/* min-w-0: a wide child (the transcript) must not stretch the page. */}
      <SidebarInset className="min-w-0">
        {/* h-14 matches the sticky offset of the audio bar on the recording page. */}
        <header className="sticky top-0 z-20 flex h-14 shrink-0 items-center gap-2 border-b bg-background/90 px-4 backdrop-blur">
          <SidebarTrigger className="max-md:size-11" />
          <Separator orientation="vertical" className="data-vertical:h-4 data-vertical:self-auto" />
          <Button
            variant="outline"
            aria-keyshortcuts="Control+K Meta+K"
            className="ml-auto max-md:h-11"
            onClick={() => setPaletteOpen(true)}
          >
            <SearchIcon data-icon="inline-start" />
            {t('common.search')}
            <Kbd aria-hidden="true" className="max-md:hidden">
              {SHORTCUT}
            </Kbd>
          </Button>
        </header>
        <Outlet />
      </SidebarInset>
      <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} />
    </SidebarProvider>
  )
}
