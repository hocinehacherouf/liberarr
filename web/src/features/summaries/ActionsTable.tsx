import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { ApiError } from '@/shared/api'
import { formatTimestamp } from '@/shared/time'
import { useToggleAction, type Action } from '@/features/recordings/api'

interface Props {
  recordingId: number
  actions: Action[]
  onSeek: (seconds: number) => void
  /** Rows to show; `index` is the position in `actions`, which the API takes. */
  visible?: (action: Action, index: number) => boolean
  /** Called on a checkbox change, before the request. */
  onToggle?: (index: number) => void
  /** Id of the element naming the table. */
  labelledBy?: string
}

export function ActionsTable({ recordingId, actions, onSeek, visible, onToggle, labelledBy }: Props) {
  const { t } = useTranslation()
  const toggle = useToggleAction(recordingId)

  if (!actions.length) return <p className="text-sm text-muted-foreground">{t('summaries.actions.none')}</p>
  // Fixed layout and the same widths in every group, so the columns line up from one table to the next.
  // min-w: scroll on a phone instead of squeezing the action.
  return (
    <Table aria-labelledby={labelledBy} className="min-w-xl table-fixed">
      <TableHeader>
        <TableRow>
          <TableHead className="w-12">{t('summaries.actions.done')}</TableHead>
          <TableHead>{t('summaries.actions.action')}</TableHead>
          <TableHead className="w-32">{t('summaries.actions.owner')}</TableHead>
          <TableHead className="w-28">{t('summaries.actions.due')}</TableHead>
          <TableHead className="w-24">{t('summaries.actions.moment')}</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {actions.map((a, index) =>
          visible?.(a, index) === false ? null : (
            <TableRow key={index} className="align-top">
              <TableCell>
                <Checkbox
                  aria-label={t('summaries.actions.markDone', { action: a.action })}
                  checked={a.done}
                  onCheckedChange={(done) => {
                    onToggle?.(index)
                    toggle.mutate(
                      { index, action: a.action, done },
                      {
                        onError: (e) =>
                          toast.error(
                            t(e instanceof ApiError && e.status === 409 ? 'summaries.actions.stale' : 'summaries.actions.updateError'),
                          ),
                      },
                    )
                  }}
                />
              </TableCell>
              <TableCell className={`whitespace-normal ${a.done ? 'text-muted-foreground line-through' : ''}`}>
                {a.action}
              </TableCell>
              <TableCell className="whitespace-normal break-words">{a.owner ?? '—'}</TableCell>
              <TableCell className="whitespace-normal text-muted-foreground">{a.due ?? '—'}</TableCell>
              <TableCell>
                {a.start != null && (
                  <Button
                    type="button"
                    variant="outline"
                    size="xs"
                    className="font-mono tabular-nums"
                    aria-label={t('summaries.actions.openAt', { action: a.action, time: formatTimestamp(a.start) })}
                    onClick={() => onSeek(a.start as number)}
                  >
                    {formatTimestamp(a.start)}
                  </Button>
                )}
              </TableCell>
            </TableRow>
          ),
        )}
      </TableBody>
    </Table>
  )
}
