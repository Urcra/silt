import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api, type ThreadSummary } from '../api'
import { useNow } from '../now'
import { keys, message } from '../query'
import { Button, Dialog, Input, Select } from '../ui'

/** Read-only links to the thread, for anyone holding one (ADR 0027); only its owner has them. */
function Links({ thread }: { thread: string }) {
  const links = useQuery({ queryKey: keys.links(thread), queryFn: () => api.links(thread), retry: false })
  const queries = useQueryClient()
  const [days, setDays] = useState(7)
  const [frozen, setFrozen] = useState(false)
  const [made, setMade] = useState<string>()
  const [error, setError] = useState<string>()
  const now = useNow()
  if (links.error) return null
  const refresh = () => queries.invalidateQueries({ queryKey: keys.links(thread) })
  return (
    <section aria-label="read-only links" className="space-y-2">
      <p className="text-xs text-fg-2">Read-only links: anyone holding one reads the conversation until it expires or you revoke it.</p>
      <ul className="space-y-1">
        {(links.data ?? [])
          .filter((link) => !link.revoked && Number(link.expires_ms) > now)
          .map((link) => (
            <li key={link.id} className="flex items-center gap-2 text-xs">
              <span>
                {link.frozen ? 'Frozen' : 'Following'}, until {new Date(Number(link.expires_ms)).toLocaleDateString()}
              </span>
              <Button variant="ghost" className="ml-auto" onClick={() => void api.revokeLink(thread, link.id).then(refresh)}>
                Revoke
              </Button>
            </li>
          ))}
      </ul>
      <form
        className="flex flex-wrap items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault()
          setError(undefined)
          void api.makeLink(thread, { days, frozen }).then(
            (link) => {
              setMade(link.url ?? undefined)
              return refresh()
            },
            (failure: unknown) => setError(message(failure)),
          )
        }}
      >
        <Select aria-label="link lasts" value={days} onChange={(event) => setDays(Number(event.target.value))}>
          <option value={1}>A day</option>
          <option value={7}>A week</option>
          <option value={30}>30 days</option>
        </Select>
        <label className="flex items-center gap-1 text-xs">
          <input type="checkbox" checked={frozen} onChange={(event) => setFrozen(event.target.checked)} />
          As it is now
        </label>
        <Button type="submit">Make a link</Button>
      </form>
      {made && (
        <p className="break-all rounded bg-surface-2 p-2 font-mono text-xs" data-testid="new-link">
          {made}
        </p>
      )}
      {error && <p className="text-xs text-danger">{error}</p>}
    </section>
  )
}

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
        {thread.local_only ? (
          <p className="text-sm text-fg-2">
            This thread stays on this machine only. To share it, choose a server for its project on the Projects page.
          </p>
        ) : status.data && !status.data.server ? (
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
            <Links thread={thread.id} />
            {error && <p className="text-xs text-danger">{error}</p>}
          </div>
        )}
      </Dialog>
    </>
  )
}
