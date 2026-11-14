import { useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useState } from 'react'
import { api, type ThreadSummary } from '../api'
import { useNow } from '../now'
import { keys, message } from '../query'
import { Button, Dialog, Input, Select, cx, toast } from '../ui'
import { Mock } from './Mock'

/**
 * A read-only link to the thread, for anyone holding one (ADR 0027); only its owner has them. With `artifact`, links to
 * that artifact alone (ADR 0066), at the version shown or following new ones.
 */
export function Links({ thread, artifact }: { thread: string; artifact?: { id: string; version: number } }) {
  const links = useQuery({ queryKey: keys.links(thread), queryFn: () => api.links(thread), retry: false })
  const queries = useQueryClient()
  const [days, setDays] = useState(7)
  const [follow, setFollow] = useState(true)
  const [made, setMade] = useState<string>()
  const [error, setError] = useState<string>()
  const now = useNow()
  if (links.error)
    return artifact ? (
      <p className="text-sm text-fg-2">
        Read-only links need a server this thread syncs with. Download the artifact as one HTML file to send it instead.
      </p>
    ) : null
  const refresh = () => queries.invalidateQueries({ queryKey: keys.links(thread) })
  const active = (links.data ?? []).filter(
    (link) => !link.revoked && Number(link.expires_ms) > now && (artifact ? link.artifact?.id === artifact.id : !link.artifact),
  )
  const make = () => {
    setError(undefined)
    const linked = artifact && { id: artifact.id, version: follow ? null : artifact.version }
    void api.makeLink(thread, { days, frozen: artifact ? false : !follow, artifact: linked ?? null }).then(
      (link) => {
        setMade(link.url ?? undefined)
        return refresh()
      },
      (failure: unknown) => setError(message(failure)),
    )
  }
  return (
    <section aria-label="read-only links" className="flex flex-col gap-2.5 rounded-md border border-line bg-bg p-3">
      <h3 className="text-sm font-medium">Read-only link</h3>
      <p className="text-xs leading-normal text-fg-2">
        {artifact
          ? 'Anyone with the link sees this artifact without an account, and nothing else of the thread. Its page runs sandboxed.'
          : 'Anyone with the link reads the conversation and the code without an account. They can’t comment or fork.'}
      </p>
      <div className="flex flex-wrap items-center gap-3">
        <label className="flex items-center gap-2 text-xs text-fg-2">
          Expires
          <Select aria-label="link lasts" value={days} onChange={(event) => setDays(Number(event.target.value))} className="text-xs">
            <option value={1}>in 1 day</option>
            <option value={7}>in 7 days</option>
            <option value={30}>in 30 days</option>
          </Select>
        </label>
        <label className="flex items-center gap-1.5 text-xs max-md:min-h-11">
          <input type="checkbox" checked={follow} onChange={(event) => setFollow(event.target.checked)} className="accent-[var(--accent)]" />
          {artifact ? 'Follow new versions' : 'Follow new turns'}
        </label>
        <Button className="ml-auto" onClick={make}>
          Make a link
        </Button>
      </div>
      {made && (
        <div className="flex gap-1.5">
          <input
            aria-label="share link"
            readOnly
            value={made}
            data-testid="new-link"
            className="min-w-0 flex-1 rounded-sm border border-line bg-surface-1 px-2 py-1 font-mono text-xs text-fg-1"
            onFocus={(event) => event.target.select()}
          />
          <Button
            onClick={() => {
              void navigator.clipboard?.writeText(made).catch(() => undefined)
              toast('Link copied', 'ok')
            }}
          >
            Copy
          </Button>
        </div>
      )}
      {made && <p className="text-2xs text-fg-2">The address shows only now: the server keeps no copy of it.</p>}
      {active.length > 0 && (
        <ul className="flex flex-col gap-1 border-t border-line-subtle pt-2">
          {active.map((link) => (
            <li key={link.id} className="flex items-center gap-2 text-xs">
              <span className="text-fg-2">
                {link.artifact
                  ? link.artifact.version === null || link.artifact.version === undefined
                    ? 'Following new versions'
                    : `Version ${link.artifact.version}`
                  : link.frozen
                    ? 'As it was'
                    : 'Following new turns'}
                , until {new Date(Number(link.expires_ms)).toLocaleDateString()}
                <Mock id="share.link-opens"> · opened 3 times</Mock>
              </span>
              <Button variant="ghost" size="sm" className="ml-auto" onClick={() => void api.revokeLink(thread, link.id).then(refresh)}>
                Revoke
              </Button>
            </li>
          ))}
        </ul>
      )}
      {error && <p className="text-xs text-danger">{error}</p>}
    </section>
  )
}

/** One choice of who sees the thread, as a card with what it means. */
function Who({ value, chosen, title, children, onChoose }: { value: string; chosen: boolean; title: string; children: ReactNode; onChoose: () => void }) {
  return (
    <label
      className={cx(
        'flex cursor-pointer items-start gap-2.5 rounded-md border px-3 py-2.5 max-md:min-h-11',
        chosen ? 'border-accent bg-accent-tint' : 'border-line hover:bg-surface-2',
      )}
    >
      <input type="radio" name="who" value={value} checked={chosen} onChange={onChoose} className="mt-0.5 accent-[var(--accent)]" />
      <span className="flex flex-col gap-0.5">
        <span className="text-sm font-medium">{title}</span>
        <span className="text-xs text-fg-2">{children}</span>
      </span>
    </label>
  )
}

/** Who sees the thread and who may write into it (ADR 0013, ADR 0014). */
export function ShareDialog({ thread }: { thread: ThreadSummary }) {
  const [open, setOpen] = useState(false)
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: open })
  const queries = useQueryClient()
  const [team, setTeam] = useState('')
  const [user, setUser] = useState('')
  const [teamOpen, setTeamOpen] = useState(false)
  const [error, setError] = useState<string>()
  const act = (action: Promise<unknown>) => {
    setError(undefined)
    return action.then(
      () => queries.invalidateQueries({ queryKey: keys.threads }),
      (failure: unknown) => setError(message(failure)),
    )
  }
  const own = thread.visibility
  // The choice shows at once, before the server answers; a refusal takes it back.
  const [picked, setPicked] = useState<string | null>(null)
  const choice = teamOpen ? 'team' : (picked ?? (own === null ? '' : own.startsWith('team:') ? 'team' : own))
  const choose = (value: string) => {
    setTeamOpen(value === 'team')
    if (value === 'team') return
    setPicked(value)
    setError(undefined)
    api.setVisibility(thread.id, value).then(
      () => queries.invalidateQueries({ queryKey: keys.threads }),
      (failure: unknown) => {
        setPicked(null)
        setError(message(failure))
      },
    )
  }
  return (
    <>
      <Button icon="link" onClick={() => setOpen(true)} aria-label="Share">
        <span className="max-md:sr-only @max-xl/head:sr-only">Share</span>
      </Button>
      <Dialog open={open} onClose={() => setOpen(false)} title={thread.title ? `Share “${thread.title}”` : 'Share this thread'}>
        {thread.local_only ? (
          <p className="text-sm text-fg-2">
            This thread stays on this machine only. To share it, choose a server for its project on the Projects page.
          </p>
        ) : status.data && !status.data.server ? (
          <p className="text-sm text-fg-2">Sharing needs a server; sign in with `silt auth login`.</p>
        ) : (
          <div className="flex flex-col gap-4 text-sm">
            <fieldset aria-label="who sees it" className="flex flex-col gap-1.5">
              <legend className="mb-1.5 font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">Who can see it</legend>
              {own === null && (
                <Who value="" chosen={choice === ''} title="As its project says" onChoose={() => setTeamOpen(false)}>
                  It follows its project’s or org’s default.
                </Who>
              )}
              <Who value="private" chosen={choice === 'private'} title={`Only ${thread.owner || 'you'}`} onChoose={() => choose('private')}>
                And the people below, plus work offered through a merge request.
              </Who>
              <Who value="team" chosen={choice === 'team'} title="A team" onChoose={() => choose('team')}>
                {own?.startsWith('team:') ? (
                  <>
                    Members of <span className="font-mono">{own.slice(5)}</span>
                  </>
                ) : (
                  'Members of one of the org’s teams.'
                )}
              </Who>
              <Who value="org" chosen={choice === 'org'} title="Everyone in the org" onChoose={() => choose('org')}>
                Forks follow this thread unless they set their own.
              </Who>
            </fieldset>
            {choice === 'team' && (
              <form
                className="flex items-center gap-2"
                onSubmit={(event) => {
                  event.preventDefault()
                  void act(api.setVisibility(thread.id, `team:${team.trim()}`)).then(() => setTeamOpen(false))
                }}
              >
                <Input
                  aria-label="team"
                  placeholder={own?.startsWith('team:') ? own.slice(5) : 'team name'}
                  value={team}
                  onChange={(event) => setTeam(event.target.value)}
                  className="flex-1"
                />
                <Button type="submit" disabled={!team.trim()}>
                  Show to this team
                </Button>
              </form>
            )}
            <div className="flex flex-col gap-2">
              <label htmlFor="share-person" className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">
                Who may also write into it
              </label>
              <ul aria-label="collaborators" className="flex flex-col gap-1">
                {thread.collaborators.length === 0 && <li className="text-xs text-fg-2">Nobody yet.</li>}
                {thread.collaborators.map((name) => (
                  <li key={name} className="flex items-center gap-2">
                    <span>{name}</span>
                    <Button variant="ghost" size="sm" className="ml-auto" onClick={() => void act(api.share(thread.id, name, true))}>
                      Remove
                    </Button>
                  </li>
                ))}
              </ul>
              <form
                className="flex items-center gap-2"
                onSubmit={(event) => {
                  event.preventDefault()
                  void act(api.share(thread.id, user.trim(), false)).then(() => setUser(''))
                }}
              >
                <Input id="share-person" aria-label="person to add" placeholder="user name" value={user} onChange={(event) => setUser(event.target.value)} className="flex-1" />
                <Button type="submit" variant="primary" disabled={!user.trim()}>
                  Add
                </Button>
              </form>
            </div>
            <Links thread={thread.id} />
            {error && <p className="text-xs text-danger">{error}</p>}
            <div className="-mx-5 -mb-4 flex justify-end border-t border-line px-5 py-3">
              <Button variant="primary" onClick={() => setOpen(false)}>
                Done
              </Button>
            </div>
          </div>
        )}
      </Dialog>
    </>
  )
}
