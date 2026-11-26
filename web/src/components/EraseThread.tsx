import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { api } from '../api'
import { keys, message } from '../query'
import { Button, Dialog, toast } from '../ui'

/** Erasing a Local only thread and its forks from this machine for good, once what goes is shown (ADR 0092). */
export function EraseThreadDialog({ thread, open, onClose }: { thread: string; open: boolean; onClose: () => void }) {
  const queries = useQueryClient()
  const navigate = useNavigate()
  const [error, setError] = useState<string>()
  const said = useQuery({
    queryKey: [...keys.thread(thread), 'erase'],
    queryFn: () => api.eraseThread(thread, false),
    enabled: open,
    gcTime: 0,
  })
  const erase = () =>
    api.eraseThread(thread, true).then(
      (done) => {
        toast(done, 'ok')
        void queries.invalidateQueries({ queryKey: keys.threads })
        onClose()
        void navigate({ to: '/' })
      },
      (failure: unknown) => setError(message(failure)),
    )
  return (
    <Dialog open={open} onClose={onClose} title="Erase from this machine">
      <div role="alertdialog" aria-label="erase this thread" className="flex flex-col gap-3 text-sm text-fg-2">
        {said.isPending ? (
          <p>Working out what goes…</p>
        ) : said.error ? (
          <p className="text-danger">{message(said.error)}</p>
        ) : (
          <p className="first-letter:uppercase">{said.data}</p>
        )}
        {error && <p className="text-danger">{error}</p>}
        <span className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="danger" disabled={!said.data} onClick={() => void erase()}>
            Erase for good
          </Button>
        </span>
      </div>
    </Dialog>
  )
}
