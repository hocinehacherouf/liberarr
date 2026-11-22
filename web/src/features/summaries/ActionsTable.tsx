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
}

export function ActionsTable({ recordingId, actions, onSeek }: Props) {
  const { t } = useTranslation()
  const toggle = useToggleAction(recordingId)

  if (!actions.length) return <p className="text-sm text-muted-foreground">{t('summaries.actions.none')}</p>
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>{t('summaries.actions.done')}</TableHead>
          <TableHead>{t('summaries.actions.action')}</TableHead>
          <TableHead>{t('summaries.actions.owner')}</TableHead>
          <TableHead>{t('summaries.actions.due')}</TableHead>
          <TableHead>{t('summaries.actions.moment')}</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {actions.map((a, index) => (
          <TableRow key={index} className="align-top">
            <TableCell>
              <Checkbox
                aria-label={t('summaries.actions.markDone', { action: a.action })}
                checked={a.done}
                onCheckedChange={(done) =>
                  toggle.mutate(
                    { index, action: a.action, done },
                    {
                      onError: (e) =>
                        toast.error(
                          t(e instanceof ApiError && e.status === 409 ? 'summaries.actions.stale' : 'summaries.actions.updateError'),
                        ),
                    },
                  )
                }
              />
            </TableCell>
            <TableCell className={`min-w-48 whitespace-normal ${a.done ? 'text-muted-foreground line-through' : ''}`}>
              {a.action}
            </TableCell>
            <TableCell>{a.owner ?? '—'}</TableCell>
            <TableCell className="text-muted-foreground">{a.due ?? '—'}</TableCell>
            <TableCell>
              {a.start != null && (
                <Button
                  type="button"
                  variant="outline"
                  size="xs"
                  className="font-mono tabular-nums"
                  onClick={() => onSeek(a.start as number)}
                >
                  {formatTimestamp(a.start)}
                </Button>
              )}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}
