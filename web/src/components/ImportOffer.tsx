import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api } from '../api'
import { useBusEvents, useTopic } from '../bus'
import { keys, message } from '../query'
import { hosted } from '../served'
import { store, stored } from '../stored'
import { Button, toast } from '../ui'

const DECLINED_KEY = 'silt-import-declined-at'

/**
 * The offer to import the agents' sessions on this machine that aren't threads yet, wherever there are some.
 * Not now hides it until more turn up, so a no on the first open doesn't hide the folders found later.
 */
export function ImportOffer() {
  const queries = useQueryClient()
  const found = useQuery({ queryKey: keys.found, queryFn: api.found, enabled: !hosted() })
  const [importing, setImporting] = useState(false)
  const [declinedAt, setDeclinedAt] = useState(() => stored(DECLINED_KEY, 0))
  const [error, setError] = useState<string>()
  // Heard from the daemon, so an import started elsewhere, or before this page opened, shows too.
  const [progress, setProgress] = useState<{ done: number; total: number }>()
  useTopic('import')
  useBusEvents((event) => {
    if (event.event === 'importing') setProgress(event.done < event.total ? { done: event.done, total: event.total } : undefined)
  })
  const waiting = (found.data ?? []).filter((folder) => folder.sessions.length > 0)
  const sessions = waiting.reduce((sum, folder) => sum + folder.sessions.reduce((count, each) => count + each.count, 0), 0)
  const running = importing || progress !== undefined
  if (!running && (sessions === 0 || sessions <= declinedAt)) return null
  const folders = waiting.length === 1 ? '1 folder' : `${waiting.length} folders`
  return (
    <div role="group" aria-label="import" className="flex flex-wrap items-center gap-3 rounded-md border border-line px-3.5 py-3">
      <span className="min-w-0 flex-[1_1_320px]">
        <b className="font-medium">
          Import the {sessions === 1 ? 'session' : `${sessions} sessions`} found in {folders}
        </b>
        <span className="block text-xs text-fg-2">
          Each becomes a thread you can read and search right away. They stay on this machine until you sync. Same as <code className="font-mono">silt import</code>.
        </span>
        {running && <ImportProgress done={progress?.done ?? 0} total={progress?.total ?? sessions} />}
        {error && <span className="block text-xs text-danger">{error}</span>}
      </span>
      <Button
        disabled={running}
        onClick={() => {
          setImporting(true)
          setError(undefined)
          api
            .importFound()
            .then(
              (made) => toast(`Imported ${made === 1 ? '1 thread' : `${made} threads`}`, 'ok'),
              (failure: unknown) => setError(message(failure)),
            )
            .finally(() => {
              setImporting(false)
              setProgress(undefined)
              // A failed session doesn't stop the others, so there can be new threads either way.
              for (const key of [keys.threads, keys.found, keys.projects]) void queries.invalidateQueries({ queryKey: key })
            })
        }}
      >
        {running ? 'Importing…' : 'Import them'}
      </Button>
      <Button
        variant="ghost"
        disabled={running}
        onClick={() => {
          setDeclinedAt(sessions)
          store(DECLINED_KEY, sessions)
        }}
      >
        Not now
      </Button>
    </div>
  )
}

function ImportProgress({ done, total }: { done: number; total: number }) {
  const percent = total > 0 ? Math.round((done / total) * 100) : 0
  return (
    <span className="mt-2 block">
      <span
        role="progressbar"
        aria-label="Importing sessions"
        aria-valuemin={0}
        aria-valuemax={total}
        aria-valuenow={done}
        className="block h-1.5 overflow-hidden rounded-full bg-line"
      >
        <span className="block h-full rounded-full bg-accent transition-[width] duration-300" style={{ width: `${percent}%` }} />
      </span>
      <span className="mt-1 block text-xs text-fg-2 tabular-nums">
        {done} of {total} sessions
      </span>
    </span>
  )
}
