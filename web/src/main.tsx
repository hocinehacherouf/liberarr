import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { BrowserRouter, Route, Routes } from 'react-router'
import './index.css'
import '@/shared/i18n'
import { LocalizedToaster } from '@/app/LocalizedToaster'
import { ThemeProvider } from '@/components/theme-provider'
import { TooltipProvider } from '@/components/ui/tooltip'
import { LoginPage } from '@/features/auth/LoginPage'
import { RequireAuth } from '@/features/auth/RequireAuth'
import { RecordingPage } from '@/features/recordings/RecordingPage'
import { RecordingsPage } from '@/features/recordings/RecordingsPage'
import { ActionsPage } from '@/features/summaries/ActionsPage'

const queryClient = new QueryClient({ defaultOptions: { queries: { retry: 1, refetchOnWindowFocus: false } } })

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <TooltipProvider>
        <QueryClientProvider client={queryClient}>
          <BrowserRouter>
            <Routes>
              <Route path="/login" element={<LoginPage />} />
              <Route element={<RequireAuth />}>
                <Route path="/" element={<RecordingsPage />} />
                <Route path="/actions" element={<ActionsPage />} />
                <Route path="/recordings/:id" element={<RecordingPage />} />
              </Route>
            </Routes>
          </BrowserRouter>
        </QueryClientProvider>
        <LocalizedToaster />
      </TooltipProvider>
    </ThemeProvider>
  </StrictMode>,
)
