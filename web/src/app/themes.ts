import { MonitorIcon, MoonIcon, SunIcon } from 'lucide-react'

/** The theme choices and their icons, shared by the sidebar menu and the command palette. */
export const THEMES = [
  ['light', SunIcon],
  ['dark', MoonIcon],
  ['system', MonitorIcon],
] as const
