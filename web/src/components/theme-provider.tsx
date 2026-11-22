import { createContext, useContext, useEffect, useState, useSyncExternalStore } from 'react'

type Theme = 'dark' | 'light' | 'system'

type ThemeProviderProps = {
  children: React.ReactNode
  defaultTheme?: Theme
  storageKey?: string
}

type ThemeProviderState = {
  theme: Theme
  setTheme: (theme: Theme) => void
  isDark: boolean
}

const ThemeProviderContext = createContext<ThemeProviderState | undefined>(undefined)

const darkQuery = window.matchMedia('(prefers-color-scheme: dark)')
const subscribe = (onChange: () => void) => {
  darkQuery.addEventListener('change', onChange)
  return () => darkQuery.removeEventListener('change', onChange)
}

const isTheme = (value: string | null): value is Theme => value === 'light' || value === 'dark' || value === 'system'

// Storage can throw (private mode, blocked cookies): the theme then just does not persist.
// A stored value that is not a theme falls back to the default.
const readStored = (key: string): Theme | null => {
  try {
    const stored = localStorage.getItem(key)
    return isTheme(stored) ? stored : null
  } catch {
    return null
  }
}

export function ThemeProvider({ children, defaultTheme = 'system', storageKey = 'liberarr-theme' }: ThemeProviderProps) {
  const [theme, setTheme] = useState<Theme>(() => readStored(storageKey) || defaultTheme)
  const systemDark = useSyncExternalStore(subscribe, () => darkQuery.matches)
  const isDark = theme === 'dark' || (theme === 'system' && systemDark)

  useEffect(() => {
    document.documentElement.classList.toggle('dark', isDark)
  }, [isDark])

  const value = {
    theme,
    isDark,
    setTheme: (next: Theme) => {
      try {
        localStorage.setItem(storageKey, next)
      } catch {
        // see readStored
      }
      setTheme(next)
    },
  }

  return <ThemeProviderContext.Provider value={value}>{children}</ThemeProviderContext.Provider>
}

export const useTheme = () => {
  const context = useContext(ThemeProviderContext)
  if (context === undefined) throw new Error('useTheme must be used within a ThemeProvider')
  return context
}

export const useIsDark = () => useTheme().isDark
