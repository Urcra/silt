import { useQuery } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { api, type AgentStatus, type Presence } from '../api'
import { keys, message } from '../query'

function AgentPicker({ value, onChange }: { value: string; onChange: (agent: string) => void }) {
  const choices = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  return (
    <select
      aria-label="agent to start"
      value={value}
      onChange={(event) => onChange(event.target.value)}
      className="rounded border border-neutral-300 px-2 py-1 text-sm"
    >
      <option value="">pick an agent…</option>
      {(choices.data?.agents ?? []).map((agent) => (
        <option key={agent.id} value={agent.id} title={agent.description}>
          {agent.name}
        </option>
      ))}
    </select>
  )
}

/** Starts an agent in a new thread of a clone this machine knows. */
export function NewAgentThread() {
  const choices = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  const navigate = useNavigate()
  const [agent, setAgent] = useState('')
  const [clone, setClone] = useState('')
  const [error, setError] = useState<string>()
  const clones = choices.data?.clones ?? []
  if (clones.length === 0) return null
  return (
    <form
      className="mb-4 flex flex-wrap items-center gap-2 text-sm"
      onSubmit={(event) => {
        event.preventDefault()
        setError(undefined)
        api.startAgent(agent, { clone: clone || clones[0].path }).then(
          (thread) => void navigate({ to: '/t/$id', params: { id: thread } }),
          (failure: unknown) => setError(message(failure)),
        )
      }}
    >
      <span>New thread with</span>
      <AgentPicker value={agent} onChange={setAgent} />
      <span>in</span>
      <select
        aria-label="clone"
        value={clone}
        onChange={(event) => setClone(event.target.value)}
        className="rounded border border-neutral-300 px-2 py-1"
      >
        {clones.map((known) => (
          <option key={known.path} value={known.path}>
            {known.repo || known.path}
          </option>
        ))}
      </select>
      <button disabled={!agent} className="rounded bg-neutral-900 px-2 py-1 text-white disabled:opacity-40">
        Start
      </button>
      {error && <p className="w-full text-rose-700">{error}</p>}
    </form>
  )
}

/**
 * The agent working in the thread from the web UI (ADR 0014): starting it,
 * sending everyone's drafts as the prompt, answering its permission requests.
 */
export function AgentPanel({
  thread,
  status,
  draft,
  onSent,
  people,
}: {
  thread: string
  status: AgentStatus | null
  draft: string
  onSent: () => void
  /** Everyone else here, whose machines may run an agent in the thread. */
  people: Presence[]
}) {
  const [agent, setAgent] = useState('')
  const [error, setError] = useState<string>()
  const act = (action: Promise<unknown>, after?: () => void) => {
    setError(undefined)
    action.then(after, (failure: unknown) => setError(message(failure)))
  }
  const running = status !== null && status.state !== 'exited'
  const elsewhere = people.find((person) => person.agent)

  return (
    <section aria-label="agent at work" className="mt-3 rounded border border-neutral-200 p-3 text-sm">
      {!running && elsewhere?.agent && (
        <div className="space-y-2">
          <p data-testid="agent-state">
            {elsewhere.user}&apos;s <span className="font-medium">{elsewhere.agent.name}</span> is {elsewhere.agent.state}
          </p>
          {elsewhere.agent.streaming && <p className="whitespace-pre-wrap">{elsewhere.agent.streaming}</p>}
          <p className="text-xs text-neutral-500">Your draft goes along with {elsewhere.user}&apos;s next prompt.</p>
        </div>
      )}
      {!running && !elsewhere && (
        <div className="flex flex-wrap items-center gap-2">
          <span>Run an agent here:</span>
          <AgentPicker value={agent} onChange={setAgent} />
          <button
            disabled={!agent}
            className="rounded bg-neutral-900 px-2 py-1 text-white disabled:opacity-40"
            onClick={() => act(api.startAgent(agent, { thread }))}
          >
            Start agent
          </button>
          {status?.error && <p className="w-full text-rose-700">{status.agent} stopped: {status.error}</p>}
        </div>
      )}
      {running && (
        <div className="space-y-2">
          <p data-testid="agent-state">
            <span className="font-medium">{status.agent}</span> is {status.state}
          </p>
          {status.thinking && <p className="text-xs italic text-neutral-500">{status.thinking}</p>}
          {status.streaming && <p className="whitespace-pre-wrap">{status.streaming}</p>}
          {status.permissions.map((asked) => (
            <div key={asked.id} className="rounded bg-amber-50 p-2" role="group" aria-label="permission">
              <p>The agent asks to run: {asked.title}</p>
              <div className="mt-1 flex gap-2">
                {asked.options.map(([option, label]) => (
                  <button
                    key={option}
                    className="rounded border border-neutral-400 px-2 py-0.5"
                    onClick={() => act(api.answer(thread, asked.id, option))}
                  >
                    {label}
                  </button>
                ))}
                <button className="text-neutral-500" onClick={() => act(api.answer(thread, asked.id, null))}>
                  Dismiss
                </button>
              </div>
            </div>
          ))}
          <div className="flex gap-2">
            <button
              disabled={status.state !== 'ready'}
              className="rounded bg-neutral-900 px-2 py-1 text-white disabled:opacity-40"
              onClick={() => act(api.prompt(thread, draft), onSent)}
              title="Sends your draft together with everyone else's"
            >
              Send
            </button>
            {status.state === 'working' && (
              <button className="rounded border border-neutral-300 px-2 py-1" onClick={() => act(api.cancelTurn(thread))}>
                Cancel turn
              </button>
            )}
            <button className="rounded border border-neutral-300 px-2 py-1" onClick={() => act(api.stopAgent(thread))}>
              Stop agent
            </button>
          </div>
          {status.error && <p className="text-rose-700">{status.error}</p>}
        </div>
      )}
      {error && <p className="text-rose-700">{error}</p>}
    </section>
  )
}
