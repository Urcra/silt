import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api, type ThreadSummary } from '../api'
import { keys, message } from '../query'
import { Button, Dialog, Input, Select } from '../ui'

/** Who sees the thread and who may write into it (ADR 0013, ADR 0014). */
export function ShareDialog({ thread }: { thread: ThreadSummary }) {
  const [open, setOpen] = useState(false)
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: open })
  const queries = useQueryClient()
  const [team, setTeam] = useState('')
  const [user, setUser] = useState('')
  const [error, setError] = useState<string>()
  const act = (action: Promise<unknown>) => {
    setError(undefined)
    return action.then(
      () => queries.invalidateQueries({ queryKey: keys.threads }),
      (failure: unknown) => setError(message(failure)),
    )
  }
  const own = thread.visibility
  const choice = own === null ? '' : own.startsWith('team:') ? 'team' : own
  return (
    <>
      <Button icon="copy" onClick={() => setOpen(true)}>
        Share
      </Button>
      <Dialog open={open} onClose={() => setOpen(false)} title="Share this thread">
        {status.data && !status.data.server ? (
          <p className="text-sm text-fg-2">Sharing needs a server; sign in with `silt auth login`.</p>
        ) : (
          <div className="space-y-4 text-sm">
            <label className="flex flex-col gap-1">
              <span className="text-xs text-fg-2">Who sees it</span>
              <Select
                aria-label="who sees it"
                value={choice}
                onChange={(event) => {
                  const chosen = event.target.value
                  if (chosen === 'team') return
                  void act(api.setVisibility(thread.id, chosen))
                }}
              >
                {own === null && <option value="">As its project says</option>}
                <option value="private">Only {thread.owner || 'you'} and the people below</option>
                <option value="org">The whole org</option>
                <option value="team">A team…</option>
              </Select>
            </label>
            {(choice === 'team' || own?.startsWith('team:')) && (
              <form
                className="flex items-center gap-2"
                onSubmit={(event) => {
                  event.preventDefault()
                  void act(api.setVisibility(thread.id, `team:${team.trim()}`))
                }}
              >
                <Input aria-label="team" placeholder={own?.startsWith('team:') ? own.slice(5) : 'team name'} value={team} onChange={(event) => setTeam(event.target.value)} />
                <Button type="submit" disabled={!team.trim()}>
                  Show to this team
                </Button>
              </form>
            )}
            <div>
              <p className="mb-1 text-xs text-fg-2">People who may also write into it</p>
              <ul aria-label="collaborators" className="space-y-1">
                {thread.collaborators.length === 0 && <li className="text-xs text-fg-3">Nobody yet.</li>}
                {thread.collaborators.map((name) => (
                  <li key={name} className="flex items-center gap-2">
                    <span>{name}</span>
                    <Button variant="ghost" className="ml-auto" onClick={() => void act(api.share(thread.id, name, true))}>
                      Remove
                    </Button>
                  </li>
                ))}
              </ul>
              <form
                className="mt-2 flex items-center gap-2"
                onSubmit={(event) => {
                  event.preventDefault()
                  void act(api.share(thread.id, user.trim(), false)).then(() => setUser(''))
                }}
              >
                <Input aria-label="person to add" placeholder="user name" value={user} onChange={(event) => setUser(event.target.value)} />
                <Button type="submit" variant="primary" disabled={!user.trim()}>
                  Add
                </Button>
              </form>
            </div>
            {error && <p className="text-xs text-danger">{error}</p>}
          </div>
        )}
      </Dialog>
    </>
  )
}
