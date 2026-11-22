import { useTranslation } from 'react-i18next'
import { THEMES } from '@/app/themes'
import { useTheme } from '@/components/theme-provider'
import { DropdownMenuRadioGroup, DropdownMenuRadioItem } from '@/components/ui/dropdown-menu'

/** The three theme choices, as the radio items of a dropdown menu. */
export function ThemeMenu() {
  const { t } = useTranslation()
  const { theme, setTheme } = useTheme()
  return (
    <DropdownMenuRadioGroup value={theme} onValueChange={(value) => setTheme(value as typeof theme)}>
      {THEMES.map(([name, Icon]) => (
        <DropdownMenuRadioItem key={name} value={name} className="max-md:min-h-11">
          <Icon />
          {t(`theme.${name}`)}
        </DropdownMenuRadioItem>
      ))}
    </DropdownMenuRadioGroup>
  )
}
