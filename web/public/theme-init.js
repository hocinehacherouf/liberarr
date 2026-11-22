// Sets the dark class before the first paint, so the page does not flash. Same logic as ThemeProvider.
;(() => {
  let stored = null
  try {
    stored = localStorage.getItem('liberarr-theme')
  } catch {}
  // Anything but "light" and "dark" (nothing stored, "system", garbage) follows the system.
  if (stored === 'dark' || (stored !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches)) {
    document.documentElement.classList.add('dark')
  }
})()
