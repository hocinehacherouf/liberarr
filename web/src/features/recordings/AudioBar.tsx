import { PauseIcon, PlayIcon } from 'lucide-react'
import { useEffect, useState, type RefObject } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Slider } from '@/components/ui/slider'
import { formatTimestamp } from '@/shared/time'

const SPEEDS = [0.75, 1, 1.25, 1.5, 2]
const SEEK_STEP_S = 5
// Space and the arrows keep their own meaning in these (typing, sliders, tabs, menus, dialogs).
const OWN_KEYS = 'input, textarea, select, button, [contenteditable], [role=dialog], [role=alertdialog], [role=menu], [role=listbox], [role=combobox], [role=tab], [role=slider]'

const togglePlay = (el: HTMLAudioElement) => {
  if (el.paused) void el.play().catch(() => {})
  else el.pause()
}

/** The sticky player of the recording page: owns the hidden audio element, which the transcript and the actions seek through `audio`. */
export function AudioBar({
  audio,
  src,
  durationHint,
}: {
  audio: RefObject<HTMLAudioElement | null>
  src: string
  durationHint: number | null
}) {
  const { t, i18n } = useTranslation()
  const [time, setTime] = useState(0)
  const [duration, setDuration] = useState(NaN)
  const [playing, setPlaying] = useState(false)
  const [rate, setRate] = useState(1)
  // Before the metadata is loaded the element reports NaN (or Infinity for some streams): use the recording's own duration.
  const total = Number.isFinite(duration) && duration > 0 ? duration : durationHint

  useEffect(() => {
    const el = audio.current
    if (!el) return
    const listeners: [string, () => void][] = [
      ['timeupdate', () => setTime(el.currentTime)],
      ['durationchange', () => setDuration(el.duration)],
      ['play', () => setPlaying(true)],
      ['pause', () => setPlaying(false)],
      ['ended', () => setPlaying(false)],
      ['ratechange', () => setRate(el.playbackRate)],
    ]
    for (const [type, fn] of listeners) el.addEventListener(type, fn)
    return () => {
      for (const [type, fn] of listeners) el.removeEventListener(type, fn)
    }
  }, [audio])

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const el = audio.current
      const target = e.target instanceof Element ? e.target : null
      if (!el || e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey || target?.closest(OWN_KEYS)) return
      if (e.key === ' ') {
        e.preventDefault() // not also a page scroll
        if (!e.repeat) togglePlay(el)
      } else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
        e.preventDefault()
        const next = el.currentTime + (e.key === 'ArrowLeft' ? -SEEK_STEP_S : SEEK_STEP_S)
        el.currentTime = Number.isFinite(el.duration) ? Math.min(Math.max(next, 0), el.duration) : Math.max(next, 0)
      }
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [audio])

  const speed = (r: number) => `${r.toLocaleString(i18n.language)}×`

  return (
    <section aria-label={t('player.label')} className="sticky bottom-0 z-10 border-t bg-background/90 backdrop-blur">
      <audio ref={audio} src={src} preload="metadata" />
      <div className="mx-auto flex w-full max-w-5xl items-center gap-2 px-4 py-2 sm:gap-3">
        <Button
          variant="ghost"
          size="icon"
          aria-label={t(playing ? 'player.pause' : 'player.play')}
          onClick={() => audio.current && togglePlay(audio.current)}
          className="max-md:size-11"
        >
          {playing ? <PauseIcon /> : <PlayIcon />}
        </Button>
        <Slider
          min={0}
          max={total ?? 1}
          step={1}
          largeStep={10}
          value={[Math.min(time, total ?? 1)]}
          disabled={total == null}
          onValueChange={(value) => {
            const seconds = typeof value === 'number' ? value : value[0]
            if (!audio.current || seconds == null) return
            audio.current.currentTime = seconds
            setTime(seconds)
          }}
          getAriaLabel={() => t('player.position')}
          getAriaValueText={(_, value) => formatTimestamp(value)}
          className="min-w-0 flex-1"
        />
        <span className="shrink-0 font-mono text-xs text-muted-foreground tabular-nums">
          {formatTimestamp(time)} / {total == null ? '--:--' : formatTimestamp(total)}
        </span>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                variant="ghost"
                size="sm"
                aria-label={`${t('player.speed')}: ${speed(rate)}`}
                className="shrink-0 tabular-nums max-md:h-11"
              />
            }
          >
            {speed(rate)}
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" side="top" className="w-auto min-w-28">
            <DropdownMenuRadioGroup
              value={String(rate)}
              onValueChange={(v) => {
                if (audio.current) audio.current.playbackRate = Number(v)
              }}
            >
              {SPEEDS.map((r) => (
                <DropdownMenuRadioItem key={r} value={String(r)} className="tabular-nums">
                  {speed(r)}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </section>
  )
}
