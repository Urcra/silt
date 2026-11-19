// Claude Code cloud sessions, started and steered through the person's own
// `claude` on the machine serving the app (ADR 0040).

import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { api, type ThreadSummary } from '../api'
import { useNow } from '../now'
import { projectName } from '../projects'
import { keys, message } from '../query'
import { hosted } from '../served'
import { ago } from '../time'
import { Button, Dot, Icon, toast } from '../ui'

/** The cloud sessions started from this machine, each linked to its thread once one records it. */
export function CloudLaunches() {
  const now = useNow()
  const launches = useQuery({ queryKey: keys.cloudLaunches, queryFn: api.cloudLaunches, enabled: !hosted() })
  if (!launches.data?.length) return null
  return (
    <section aria-labelledby="cloud-sessions" className="flex flex-col gap-1.5">
      <h2 id="cloud-sessions" className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">
        Cloud sessions
      </h2>
      <ul className="flex flex-col gap-1.5">
        {[...launches.data].reverse().map((launch) => (
          <li
            key={launch.id}
            className="flex flex-wrap items-center gap-x-3 gap-y-0.5 rounded-md border border-line bg-elevated px-3 py-2 text-sm"
          >
            <Dot tone={launch.thread ? 'ok' : 'warn'} />
            {launch.thread ? (
              <Link to="/t/$id" params={{ id: launch.thread }} className="min-w-0 flex-1 truncate text-fg-1 hover:text-accent-fg">
                {launch.task}
              </Link>
            ) : (
              <span className="min-w-0 flex-1 truncate">{launch.task}</span>
            )}
            <span className="font-mono text-2xs text-fg-2">
              {projectName(launch.repo)} · {launch.thread ? 'recording' : 'waiting for its thread'} ·{' '}
              <span data-volatile>{ago(launch.started_ms, now)}</span>
            </span>
            <a href={launch.url} target="_blank" rel="noreferrer" className="text-2xs text-accent-fg">
              claude.ai
            </a>
          </li>
        ))}
      </ul>
    </section>
  )
}

/** A cloud thread's link to claude.ai, and on the machine serving the app, a reply box that queues a message into the session. */
export function CloudSession({ thread }: { thread: ThreadSummary }) {
  const [text, setText] = useState('')
  const [sending, setSending] = useState(false)
  if (!thread.cloud) return null
  const url = `https://claude.ai/code/${thread.cloud}`
  // Resumed here, the thread follows the session on this machine, not the cloud one.
  const steerable = !hosted() && thread.location !== 'here'
  const send = () => {
    setSending(true)
    api
      .cloudSend(thread.id, text.trim())
      .then(
        () => {
          toast('Sent to the cloud session; its reply arrives as the next turn', 'ok')
          setText('')
        },
        (failure: unknown) => toast(message(failure), 'danger'),
      )
      .finally(() => setSending(false))
  }
  return (
    <section aria-label="cloud session" className="flex flex-col gap-2 rounded-md border border-line bg-elevated px-3 py-2.5">
      <p className="flex items-center gap-1.5 text-xs text-fg-2">
        <Icon name="cloud" className="h-3.5 w-3.5" />
        Runs as a Claude Code cloud session.
        <a href={url} target="_blank" rel="noreferrer" className="text-accent-fg">
          Open on claude.ai
        </a>
      </p>
      {steerable && (
        <form
          className="flex items-end gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            if (text.trim() && !sending) send()
          }}
        >
          <textarea
            aria-label="message to the cloud session"
            rows={2}
            value={text}
            onChange={(event) => setText(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && (event.metaKey || event.ctrlKey) && text.trim() && !sending) {
                event.preventDefault()
                send()
              }
            }}
            placeholder="Send the cloud session a follow-up"
            className="min-w-0 flex-1 resize-none rounded-sm border border-line bg-surface px-2.5 py-1.5 text-sm placeholder:text-fg-2 focus:outline-none"
          />
          <Button type="submit" variant="primary" disabled={!text.trim() || sending}>
            {sending ? 'Sending…' : 'Send'}
          </Button>
        </form>
      )}
    </section>
  )
}
