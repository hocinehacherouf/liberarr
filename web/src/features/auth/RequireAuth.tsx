import { Navigate } from 'react-router'
import { AppShell } from '@/app/AppShell'
import { Loading } from '@/shared/Loading'
import { useMe } from '@/features/auth/api'

/** Gate for every page except login. */
export function RequireAuth() {
  const me = useMe()

  if (me.isLoading) return <Loading className="p-8" />
  if (!me.data) return <Navigate to="/login" replace />

  return <AppShell />
}
