import { useTranslation } from 'react-i18next'
import { ApiError } from '../../shared/api'
import { formatTimestamp } from '../../shared/time'
import { useToggleAction, type Action } from '../recordings/api'

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
    <div className="-mx-4 overflow-x-auto px-4 sm:-mx-5 sm:px-5">
      {toggle.isError && (
        <p role="alert" className="mb-2 text-sm text-destructive">
          {t(
            toggle.error instanceof ApiError && toggle.error.status === 409
              ? 'summaries.actions.stale'
              : 'summaries.actions.updateError',
          )}
        </p>
      )}
      <table className="w-full text-sm">
        <thead className="text-left text-xs text-muted-foreground">
          <tr>
            <th className="px-2 pb-2 font-medium">{t('summaries.actions.done')}</th>
            <th className="px-2 pb-2 font-medium">{t('summaries.actions.action')}</th>
            <th className="px-2 pb-2 font-medium">{t('summaries.actions.owner')}</th>
            <th className="px-2 pb-2 font-medium">{t('summaries.actions.due')}</th>
            <th className="px-2 pb-2 font-medium">{t('summaries.actions.moment')}</th>
          </tr>
        </thead>
        <tbody>
          {actions.map((a, index) => (
            <tr key={index} className="border-t border-border align-top">
              <td className="p-2">
                <input
                  type="checkbox"
                  className="size-4 align-middle"
                  aria-label={t('summaries.actions.markDone', { action: a.action })}
                  checked={a.done}
                  onChange={(e) => toggle.mutate({ index, action: a.action, done: e.target.checked })}
                />
              </td>
              <td className={`min-w-48 p-2 ${a.done ? 'text-muted-foreground line-through' : ''}`}>{a.action}</td>
              <td className="p-2 whitespace-nowrap">{a.owner ?? '—'}</td>
              <td className="p-2 whitespace-nowrap text-muted-foreground">{a.due ?? '—'}</td>
              <td className="p-2">
                {a.start != null && (
                  <button
                    className="rounded border border-primary/40 px-1.5 py-0.5 font-mono text-xs text-primary tabular-nums hover:bg-primary/10"
                    onClick={() => onSeek(a.start as number)}
                  >
                    {formatTimestamp(a.start)}
                  </button>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
