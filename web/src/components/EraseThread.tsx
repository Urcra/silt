import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { api, type ThreadSummary } from '../api'
import { keys, message } from '../query'
import { workspaceOf } from '../rules'
import { hosted } from '../served'
import { Button, toast } from '../ui'

/** Erasing a Local only thread and its forks from this machine for good, once what goes is shown (ADR 0072). */
export function EraseThread({ thread, summary, onDone }: { thread: string; summary: ThreadSummary | undefined; onDone: () => void }) {
  const queries = useQueryClient()
  const navigate = useNavigate()
  const workspaces = useQuery({
    queryKey: keys.workspaces,
    queryFn: api.workspaces,
    enabled: !hosted(),
  })
  const [said, setSaid] = useState<string>()
  const [error, setError] = useState<string>()
  const localOnly = summary && workspaceOf(workspaces.data, summary.repo, summary.workspace)?.effective.local_only
  if (hosted() || !localOnly) return null
  if (said === undefined) {
    return (
      <Button
        variant="ghost"
        className="justify-start font-normal text-danger"
        onClick={() => api.eraseThread(thread, false).then(setSaid, (failure: unknown) => setError(message(failure)))}
      >
        Erase from this machine…
        {error && <span className="text-xs">{error}</span>}
      </Button>
    )
  }
  return (
    <div role="alertdialog" aria-label="erase this thread" className="flex flex-col gap-2 rounded-sm border border-line p-2 text-xs text-fg-2">
      <p className="first-letter:uppercase">{said}</p>
      <span className="flex justify-end gap-2">
        <Button variant="ghost" size="sm" onClick={() => setSaid(undefined)}>
          Cancel
        </Button>
        <Button
          variant="danger"
          size="sm"
          onClick={() =>
            api.eraseThread(thread, true).then(
              (done) => {
                toast(done, 'ok')
                void queries.invalidateQueries({ queryKey: keys.threads })
                onDone()
                void navigate({ to: '/' })
              },
              (failure: unknown) => setError(message(failure)),
            )
          }
        >
          Erase for good
        </Button>
      </span>
      {error && <p className="text-danger">{error}</p>}
    </div>
  )
}
