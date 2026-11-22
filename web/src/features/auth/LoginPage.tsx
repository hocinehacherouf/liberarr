import type { TFunction } from 'i18next'
import { TriangleAlertIcon } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { Navigate } from 'react-router'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Field, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { ApiError } from '@/shared/api'
import { Logo } from '@/shared/Logo'
import { useLogin, useMe } from '@/features/auth/api'

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

  return (
    <main className="mx-auto flex min-h-screen max-w-sm flex-col justify-center gap-6 px-4">
      <h1 className="flex justify-center text-3xl">
        <Logo />
      </h1>
      <Card>
        <CardContent>
          <form onSubmit={submit}>
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="username">{t('auth.username')}</FieldLabel>
                <Input
                  id="username"
                  className="max-md:h-11"
                  autoComplete="username"
                  required
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="password">{t('auth.password')}</FieldLabel>
                <Input
                  id="password"
                  className="max-md:h-11"
                  type="password"
                  autoComplete="current-password"
                  required
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                />
              </Field>
              {login.isError && (
                <Alert variant="destructive" role="alert">
                  <TriangleAlertIcon />
                  <AlertDescription>{message(login.error, t)}</AlertDescription>
                </Alert>
              )}
              <Button type="submit" disabled={login.isPending} className="w-full max-md:h-11">
                {t('auth.signIn')}
              </Button>
            </FieldGroup>
          </form>
        </CardContent>
      </Card>
    </main>
  )
}
