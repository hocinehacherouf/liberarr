import { useTranslation } from 'react-i18next'
import { Link, Navigate, Outlet } from 'react-router'
import { Loading } from '../../shared/Loading'
import { Logo } from '../../shared/Logo'
import { useLogout, useMe } from './api'

/** Gate and shell for every page except login. */
export function RequireAuth() {
  const { t } = useTranslation()
  const me = useMe()
  const logout = useLogout()

  if (me.isLoading) return <Loading className="p-8" />
  if (!me.data) return <Navigate to="/login" replace />

  return (
    <>
      <header className="sticky top-0 z-20 border-b border-border bg-card/90 backdrop-blur">
        <div className="bg-brand h-0.5" />
        <div className="mx-auto flex max-w-5xl items-center justify-between px-4 py-2.5">
          <Link to="/" className="rounded text-lg">
            <Logo />
          </Link>
          <button
            onClick={() => logout.mutate()}
            className="rounded-md border border-border px-2.5 py-1 text-sm text-muted-foreground hover:border-primary hover:text-foreground"
          >
            {t('auth.signOut')}
          </button>
        </div>
      </header>
      <Outlet />
    </>
  )
}
