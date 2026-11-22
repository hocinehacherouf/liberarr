// Sets the dark class before the first paint, so the page does not flash. Same logic as ThemeProvider.
var stored = null
try {
  stored = localStorage.getItem('liberarr-theme')
} catch {}
if (stored === 'dark' || ((!stored || stored === 'system') && matchMedia('(prefers-color-scheme: dark)').matches)) {
  document.documentElement.classList.add('dark')
}
