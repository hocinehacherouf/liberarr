import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'
import en from '@/shared/locales/en.json'
import fr from '@/shared/locales/fr.json'

// Compile-time check: a key missing from the French catalogue is a build error.
fr satisfies typeof en

const SUPPORTED = ['en', 'fr']

/** The first supported browser language, else English. */
function initialLanguage(): string {
  for (const tag of navigator.languages) {
    const primary = tag.split('-')[0].toLowerCase()
    if (SUPPORTED.includes(primary)) return primary
  }
  return 'en'
}

void i18n.use(initReactI18next).init({
  resources: { en: { translation: en }, fr: { translation: fr } },
  lng: initialLanguage(),
  fallbackLng: 'en',
  interpolation: { escapeValue: false },
})

const syncHtmlLang = (lng: string) => document.documentElement.setAttribute('lang', lng)
syncHtmlLang(i18n.language)
i18n.on('languageChanged', syncHtmlLang)

export default i18n
