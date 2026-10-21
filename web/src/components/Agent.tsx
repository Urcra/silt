import { useQuery } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { rememberModes } from '../agentModes'
import { api, type AgentStatus, type Presence, type Preset, type PromptAttachment } from '../api'
import { readAttachment } from '../attachments'
import { APPROVED, PRESETS, STATE_PHRASE, running } from '../agentState'
import { keys, message } from '../query'
import { hosted } from '../served'
import { useActions } from '../shortcuts'
import { PromptQueue } from './Queue'

export function AgentPicker({ value, onChange }: { value: string; onChange: (agent: string) => void }) {
  const choices = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  return (
    <select
      aria-label="agent to start"
      value={value}
      onChange={(event) => onChange(event.target.value)}
      className="rounded border border-line px-2 py-1 text-sm"
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

/** Which of the agent's permission requests the daemon answers by itself (ADR 0022). */
function PresetPicker({ thread, preset, onPreset }: { thread: string; preset: Preset; onPreset: (preset: Preset) => void }) {
  return (
    <label className="flex items-center gap-2 text-xs text-fg-2">
      Permissions
      <select
        aria-label="permissions"
        value={preset}
        onChange={(event) => {
          const chosen = event.target.value as Preset
          void api.setPreset(thread, chosen).then(() => onPreset(chosen))
        }}
        className="rounded-sm border border-line bg-elevated px-1.5 py-0.5 text-xs text-fg-1"
      >
        {PRESETS.map((choice) => (
          <option key={choice.id} value={choice.id}>
            {choice.label}
          </option>
        ))}
      </select>
      {preset === 'full_access' && <span className="text-warn">nothing asks; the agent's own sandbox is all that limits it</span>}
    </label>
  )
}

/** Whether a prompt goes straight to the agent rather than its queue. */
function ready(status: AgentStatus): boolean {
  return status.state === 'idle' || status.state === 'awaiting_input'
}

const SELECT ='rounded-sm border border-line bg-elevated px-1.5 py-0.5 text-xs text-fg-1'

/** The agent's own modes and settings, such as its model, as it offers them. */
function AgentChoices({ thread, status, act }: { thread: string; status: AgentStatus; act: (action: Promise<unknown>) => void }) {
  return (
    <div className="flex flex-wrap items-center gap-3 text-xs text-fg-2">
      {status.modes && (
        <label className="flex items-center gap-2">
          Mode
          <select
            aria-label="mode"
            value={status.modes.current}
            onChange={(event) => act(api.setMode(thread, event.target.value))}
            className={SELECT}
          >
            {status.modes.available.map((mode) => (
              <option key={mode.id} value={mode.id} title={mode.description ?? undefined}>
                {mode.name}
              </option>
            ))}
          </select>
        </label>
      )}
      {status.settings.map((setting) => (
        <label key={setting.id} className="flex items-center gap-2">
          {setting.name}
          <select
            aria-label={setting.name.toLowerCase()}
            value={setting.current}
            onChange={(event) => act(api.setSetting(thread, setting.id, event.target.value))}
            className={SELECT}
          >
            {setting.choices.map((choice) => (
              <option key={choice.id} value={choice.id} title={choice.description ?? undefined}>
                {choice.name}
              </option>
            ))}
          </select>
        </label>
      ))}
    </div>
  )
}

/**
 * The agent working in the thread from the web UI (ADR 0014): starting it,
 * sending everyone's drafts as the prompt, answering its permission requests.
 */
export function AgentPanel({
  thread,
  status,
  toolOutput,
  preset,
  onPreset,
  draft,
  onSent,
  people,
}: {
  thread: string
  status: AgentStatus | null
  /** What the running tool printed so far. */
  toolOutput: string
  preset: Preset
  onPreset: (preset: Preset) => void
  draft: string
  onSent: () => void
  /** Everyone else here, whose machines may run an agent in the thread. */
  people: Presence[]
}) {
  const [agent, setAgent] = useState('')
  const [planFirst, setPlanFirst] = useState(false)
  const [attached, setAttached] = useState<PromptAttachment[]>([])
  const [error, setError] = useState<string>()
  const act = (action: Promise<unknown>, after?: () => void) => {
    setError(undefined)
    action.then(after, (failure: unknown) => setError(message(failure)))
  }
  const live = status !== null && running(status.state)
  const elsewhere = people.find((person) => person.agent)
  const modes = status?.modes?.available
  const agentName = status?.agent
  useEffect(() => {
    if (agentName && modes) rememberModes(agentName, modes)
  }, [agentName, modes])
  const working = status?.state === 'working' || status?.state === 'awaiting_permission'
  useActions([
    ...(live ? [{ id: 'stop', label: `Stop ${status.agent}`, run: () => act(api.stopAgent(thread)) }] : []),
    ...(working ? [{ id: 'cancel', label: 'Cancel the turn', run: () => act(api.cancelTurn(thread)) }] : []),
    ...(status?.state === 'awaiting_input'
      ? [{ id: 'approve', label: 'Approve the plan', run: () => act(api.prompt(thread, APPROVED)) }]
      : []),
  ])

  return (
    <section aria-label="agent at work" className="mt-3 rounded border border-line p-3 text-sm">
      {!live && elsewhere?.agent && (
        <div className="space-y-2">
          <p data-testid="agent-state">
            {elsewhere.user}&apos;s <span className="font-medium">{elsewhere.agent.name}</span>{' '}
            {STATE_PHRASE[elsewhere.agent.state]}
          </p>
          {elsewhere.agent.streaming && <p className="whitespace-pre-wrap">{elsewhere.agent.streaming}</p>}
          <p className="text-xs text-fg-3">Your draft goes along with {elsewhere.user}&apos;s next prompt.</p>
        </div>
      )}
      {!live && !elsewhere && hosted() && (
        <p className="text-fg-2">No device of yours runs an agent in this thread right now.</p>
      )}
      {!live && !elsewhere && !hosted() && (
        <div className="flex flex-wrap items-center gap-2">
          <span>Run an agent here:</span>
          <AgentPicker value={agent} onChange={setAgent} />
          <button
            disabled={!agent}
            className="rounded bg-accent px-2 py-1 text-on-accent disabled:opacity-40"
            onClick={() => act(api.startAgent(agent, { thread }))}
          >
            Start agent
          </button>
          {status?.error && <p className="w-full text-danger">{status.agent} stopped: {status.error}</p>}
        </div>
      )}
      {live && (
        <div className="space-y-2">
          <p data-testid="agent-state">
            <span className="font-medium">{status.agent}</span> {STATE_PHRASE[status.state]}
          </p>
          {status.thinking && <p className="text-xs italic text-fg-3">{status.thinking}</p>}
          {status.streaming && <p className="whitespace-pre-wrap">{status.streaming}</p>}
          {toolOutput && (
            <pre data-testid="tool-output" className="max-h-48 overflow-auto rounded bg-surface-1 p-2 text-xs">
              {toolOutput}
            </pre>
          )}
          {status.permissions.map((asked) => (
            <div key={asked.id} className="rounded bg-warn-tint p-2" role="group" aria-label="permission">
              <p>The agent asks to run: {asked.title}</p>
              <div className="mt-1 flex gap-2">
                {asked.options.map(([option, label]) => (
                  <button
                    key={option}
                    className="rounded border border-line-strong px-2 py-0.5"
                    onClick={() => act(api.answer(thread, asked.id, option))}
                  >
                    {label}
                  </button>
                ))}
                <button className="text-fg-3" onClick={() => act(api.answer(thread, asked.id, null))}>
                  Dismiss
                </button>
              </div>
            </div>
          ))}
          {status.state === 'awaiting_input' && (
            <div className="flex items-center gap-2 rounded bg-accent-tint p-2" role="group" aria-label="plan">
              <span>Approve the plan, or send a draft to change it.</span>
              <button
                className="rounded bg-accent px-2 py-0.5 text-on-accent"
                onClick={() => act(api.prompt(thread, APPROVED))}
              >
                Approve plan
              </button>
            </div>
          )}
          <AgentChoices thread={thread} status={status} act={act} />
          {attached.length > 0 && (
            <ul aria-label="attached files" className="flex flex-wrap gap-2 text-xs">
              {attached.map((file, index) => (
                <li key={`${file.name}-${index}`} className="flex items-center gap-1 rounded-sm bg-surface-2 px-1.5 py-0.5">
                  {file.name}
                  <button
                    aria-label={`remove ${file.name}`}
                    className="text-fg-3 hover:text-fg-1"
                    onClick={() => setAttached(attached.filter((_, kept) => kept !== index))}
                  >
                    ×
                  </button>
                </li>
              ))}
            </ul>
          )}
          <div className="flex flex-wrap items-center gap-2">
            <button
              disabled={!draft.trim() || (attached.length > 0 && !ready(status))}
              className="rounded bg-accent px-2 py-1 text-on-accent disabled:opacity-40"
              onClick={() => act(api.prompt(thread, draft, planFirst && ready(status), attached), () => {
                setPlanFirst(false)
                setAttached([])
                onSent()
              })}
              title={
                ready(status)
                  ? "Sends your draft together with everyone else's"
                  : attached.length > 0
                    ? 'Attached files go with a prompt the agent takes now'
                    : 'Queues your draft for when the agent is ready'
              }
            >
              {ready(status) ? 'Send' : 'Queue'}
            </button>
            {ready(status) && (
              <label
                className="flex items-center gap-1 text-xs text-fg-2"
                title="The agent writes a plan for you to approve before it edits anything"
              >
                <input type="checkbox" checked={planFirst} onChange={(event) => setPlanFirst(event.target.checked)} />
                Plan first
              </label>
            )}
            <label
              hidden={hosted()}
              className="cursor-pointer rounded border border-line px-2 py-1 text-xs"
              title={status.takes_images ? 'Attach images or files' : `Attach text files; ${status.agent} takes no images`}
            >
              Attach
              <input
                type="file"
                multiple
                aria-label="attach files"
                className="sr-only"
                accept={status.takes_images || status.takes_files ? undefined : 'text/*,.md,.json,.ts,.tsx,.rs,.py,.toml,.yaml,.yml'}
                onChange={(event) => {
                  const files = [...(event.target.files ?? [])]
                  event.target.value = ''
                  act(Promise.all(files.map(readAttachment)).then((read) => setAttached((before) => [...before, ...read])))
                }}
              />
            </label>
            {(status.state === 'working' || status.state === 'awaiting_permission') && (
              <button
                disabled={!draft.trim()}
                className="rounded border border-line px-2 py-1 disabled:opacity-40"
                onClick={() => act(api.steer(thread, draft), onSent)}
                title="Interrupts the agent's turn to tell it your draft"
              >
                Steer now
              </button>
            )}
            {(status.state === 'working' || status.state === 'awaiting_permission') && (
              <button className="rounded border border-line px-2 py-1" onClick={() => act(api.cancelTurn(thread))}>
                Cancel turn
              </button>
            )}
            <button className="rounded border border-line px-2 py-1" onClick={() => act(api.stopAgent(thread))}>
              Stop agent
            </button>
          </div>
          {status.error && <p className="text-danger">{status.error}</p>}
        </div>
      )}
      {error && <p className="text-danger">{error}</p>}
      {(live || !hosted()) && (
        <div className="mt-2">
          <PresetPicker thread={thread} preset={preset} onPreset={onPreset} />
        </div>
      )}
      {!hosted() && <PromptQueue thread={thread} />}
    </section>
  )
}
