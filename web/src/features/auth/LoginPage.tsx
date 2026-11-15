import type { TFunction } from 'i18next'
import { useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { Navigate } from 'react-router'
import { ApiError } from '../../shared/api'
import { Logo } from '../../shared/Logo'
import { useLogin, useMe } from './api'

function message(e: Error, t: TFunction): string {
  if (e instanceof ApiError && e.status === 401) return t('auth.errors.invalidCredentials')
  if (e instanceof ApiError && e.status === 429) return t('auth.errors.tooManyAttempts')
  return t('auth.errors.generic')
}

export function LoginPage() {
  const { t } = useTranslation()
  const me = useMe()
  const login = useLogin()
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')

  if (me.data) return <Navigate to="/" replace />

  const submit = (e: FormEvent) => {
    e.preventDefault()
    login.mutate({ username, password })
  }
  const field = 'mt-1 w-full rounded-md border border-line bg-canvas px-3 py-2 text-fg focus:border-accent'

  return (
    <main className="mx-auto flex min-h-screen max-w-sm flex-col justify-center px-4">
      <h1 className="mb-6 flex justify-center text-3xl">
        <Logo />
      </h1>
      <form onSubmit={submit} className="space-y-4 overflow-hidden rounded-xl border border-line bg-surface p-6 shadow-sm">
        <div aria-hidden="true" className="bg-brand -mx-6 -mt-6 mb-6 h-1" />
        <label className="block text-sm font-medium text-muted">
          {t('auth.username')}
          <input
            className={field}
            autoComplete="username"
            required
            value={username}
            onChange={(e) => setUsername(e.target.value)}
          />
        </label>
        <label className="block text-sm font-medium text-muted">
          {t('auth.password')}
          <input
            className={field}
            type="password"
            autoComplete="current-password"
            required
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </label>
        {login.isError && (
          <p role="alert" className="flex gap-2 text-sm text-err">
            <span aria-hidden="true">▲</span>
            {message(login.error, t)}
          </p>
        )}
        <button
          type="submit"
          disabled={login.isPending}
          className="bg-brand-strong w-full rounded-md px-3 py-2 text-sm font-semibold text-white hover:brightness-110 disabled:opacity-60"
        >
          {t('auth.signIn')}
        </button>
      </form>
    </main>
  )
}
