import { useEffect, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'

// Fixed list, picked by speaker index: the same speaker keeps the same colour on every row.
const COLORS = [
  'text-sky-700 dark:text-sky-300',
  'text-amber-700 dark:text-amber-300',
  'text-emerald-700 dark:text-emerald-300',
  'text-rose-700 dark:text-rose-300',
  'text-violet-700 dark:text-violet-300',
  'text-teal-700 dark:text-teal-300',
  'text-orange-700 dark:text-orange-300',
  'text-fuchsia-700 dark:text-fuchsia-300',
]
const MAX_NAME = 60

/** The speaker's name; click it to rename (blank resets to "Speaker N"). */
export function SpeakerLabel({
  speaker,
  label,
  onRename,
}: {
  speaker: number
  label: string
  onRename: (speaker: number, name: string) => Promise<unknown>
}) {
  const { t } = useTranslation()
  const [editing, setEditing] = useState(false)
  const [name, setName] = useState('')
  const [pending, setPending] = useState(false)
  const button = useRef<HTMLButtonElement>(null)
  const wasEditing = useRef(false)

  // Hand the focus back to the label when the form closes, or keyboard users lose their place.
  useEffect(() => {
    if (wasEditing.current && !editing) button.current?.focus()
    wasEditing.current = editing
  }, [editing])

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    // The unedited default label ("Speaker 1") must not be stored as a name.
    if (name === label) return setEditing(false)
    setPending(true)
    try {
      await onRename(speaker, name)
      setEditing(false)
    } catch {
      toast.error(t('speakers.renameError'))
    } finally {
      setPending(false)
    }
  }

  if (!editing)
    return (
      <Button
        ref={button}
        type="button"
        variant="ghost"
        size="xs"
        title={t('speakers.rename')}
        onClick={() => {
          setName(label)
          setEditing(true)
        }}
        className={`shrink-0 font-semibold ${COLORS[speaker % COLORS.length]}`}
      >
        {label}
      </Button>
    )

  return (
    <form
      onSubmit={(e) => void submit(e)}
      onKeyDown={(e) => e.key === 'Escape' && setEditing(false)}
      className="flex flex-wrap items-center gap-1"
    >
      <Input
        autoFocus
        value={name}
        onChange={(e) => setName(e.target.value)}
        maxLength={MAX_NAME}
        aria-label={t('speakers.nameOf', { label })}
        className="h-7 w-36"
      />
      <Button type="submit" size="xs" disabled={pending}>
        {t('speakers.save')}
      </Button>
      <Button type="button" variant="outline" size="xs" onClick={() => setEditing(false)}>
        {t('speakers.cancel')}
      </Button>
    </form>
  )
}
