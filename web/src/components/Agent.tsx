import { useQuery } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { rememberModes } from '../agentModes'
import { api, type AgentStatus, type PermissionAsked, type Presence, type Preset, type PromptAttachment } from '../api'
import { readAttachment } from '../attachments'
import { APPROVED, DEFAULT_PRESETS, PRESETS, STATE_PHRASE, running } from '../agentState'
import { useNow } from '../now'
import { tokens } from '../transcript'
import { keys, message } from '../query'
import { hosted } from '../served'
import { formatKeys, useActions, useShortcut } from '../shortcuts'
import { Button, Dot, Icon, Kbd, Spinner, cx } from '../ui'
import { ChipSelect, GHOST_CHIP } from './ChipSelect'
import { ComposerBox, RoundButton } from './Composer'
import { Markdown } from './Markdown'
import { PromptQueue } from './Queue'

export function AgentPicker({ value, onChange, className }: { value: string; onChange: (agent: string) => void; className?: string }) {
  const choices = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  return (
    <select
      aria-label="agent to start"
      value={value}
      onChange={(event) => onChange(event.target.value)}
      className={className ?? 'rounded-sm border border-line bg-elevated px-2 py-1 text-sm text-fg-1'}
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

/** Whether a prompt goes straight to the agent rather than its queue. */
function ready(status: AgentStatus): boolean {
  return status.state === 'idle' || status.state === 'awaiting_input'
}

/** Whether a turn is under way. */
function working(status: AgentStatus | null): boolean {
  return status?.state === 'working' || status?.state === 'awaiting_permission'
}

/** A permission the agent asks for, answered in place: ⏎ allows, Esc dismisses (ADR 0022). */
export function PermissionCard({ thread, asked, agent, first }: { thread: string; asked: PermissionAsked; agent: string; first: boolean }) {
  const [error, setError] = useState<string>()
  const answer = (option: string | null) => {
    setError(undefined)
    api.answer(thread, asked.id, option).catch((failure: unknown) => setError(message(failure)))
  }
  const [allow] = asked.options
  useShortcut('enter', 'Allow what the agent asks', () => {
    if (first && allow && !document.querySelector('dialog[open]')) answer(allow[0])
  })
  useShortcut('escape', 'Dismiss what the agent asks', () => {
    if (first && !document.querySelector('dialog[open]')) answer(null)
  })
  return (
    <div role="group" aria-label="permission" className="flex flex-col gap-2.5 rounded-md border border-warn bg-elevated px-3.5 py-3">
      <span className="flex items-center gap-2 text-xs text-fg-2">
        <span className="h-1.5 w-1.5 rounded-full bg-warn" />
        {agent} asks to run
      </span>
      <code className="rounded-sm border border-line bg-bg px-2.5 py-2 font-mono text-xs break-all text-fg-1">{asked.title}</code>
      <span className="flex flex-wrap gap-1.5">
        {asked.options.map(([option, label], index) => (
          <Button key={option} variant={index === 0 ? 'primary' : 'secondary'} onClick={() => answer(option)}>
            {label}
            {index === 0 && first && <Kbd onAccent>⏎</Kbd>}
          </Button>
        ))}
        <Button variant="ghost" onClick={() => answer(null)}>
          Dismiss {first && <Kbd>Esc</Kbd>}
        </Button>
      </span>
      {error && <span className="text-xs text-danger">{error}</span>}
    </div>
  )
}

/** The agent's plan, waiting for a yes or a change; approving it moves the thread off Plan (ADR 0044). */
export function PlanCard({ thread, preset, onPreset }: { thread: string; preset: Preset; onPreset: (preset: Preset) => void }) {
  const [error, setError] = useState<string>()
  const [after, setAfter] = useState<Preset>('ask_before_commands')
  const approve = () => {
    setError(undefined)
    const moving = preset === 'plan' ? api.setPreset(thread, after).then(() => onPreset(after)) : Promise.resolve()
    moving.then(() => api.prompt(thread, APPROVED)).catch((failure: unknown) => setError(message(failure)))
  }
  return (
    <section role="group" aria-label="plan" className="flex flex-col gap-2.5 rounded-md border border-warn bg-elevated px-3.5 py-3">
      <span className="flex items-center gap-2 text-xs text-fg-2">
        <span className="h-1.5 w-1.5 rounded-full bg-warn" />
        Plan to approve · nothing is edited until you do; or write what to change below
      </span>
      <span className="flex flex-wrap items-center gap-1.5">
        <Button variant="primary" onClick={approve}>
          Approve plan
        </Button>
        {preset === 'plan' && (
          <ChipSelect
            label="after approving, the preset"
            prefix="then"
            icon="lock"
            value={after}
            onChange={(chosen) => setAfter(chosen as Preset)}
            options={DEFAULT_PRESETS.map((choice) => ({ id: choice.id, name: choice.label, description: choice.says }))}
          />
        )}
      </span>
      {error && <span className="text-xs text-danger">{error}</span>}
    </section>
  )
}

/** How long the turn under way has run, as `0:42`. */
function useTurnClock(status: AgentStatus | null): string | null {
  const now = useNow(1000)
  const [started, setStarted] = useState<number | null>(null)
  const turning = working(status)
  if (turning && started === null) setStarted(now)
  if (!turning && started !== null) setStarted(null)
  if (started === null) return null
  const seconds = Math.max(0, Math.floor((now - started) / 1000))
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}

/** Esc stops the turn while the agent works and asks nothing; a question or a permission takes Esc while open. */
function StopKey({ onCancel }: { onCancel: () => void }) {
  useShortcut('escape', 'Stop the turn', () => {
    if (!document.querySelector('dialog[open]')) onCancel()
  })
  return <Kbd>Esc</Kbd>
}

/** What the agent is doing now, before it is recorded: its thinking, its words, the running tool's output, and how long it's been at it. */
export function AgentTail({ status, toolOutput, onCancel }: { status: AgentStatus; toolOutput: string; onCancel: () => void }) {
  const clock = useTurnClock(status)
  if (!working(status) && !status.streaming) return null
  const thinkingNow = status.state === 'working' && !status.streaming && !toolOutput
  return (
    <div className="flex flex-col gap-3" aria-live="polite">
      {status.thinking && (
        // Open while it's what the agent does; folded once it answers, runs a tool or asks.
        <details key={thinkingNow ? 'now' : 'done'} open={thinkingNow} className="group/think border-l-2 border-line pl-3">
          <summary className="flex cursor-pointer list-none items-center gap-1.5 text-xs text-fg-2">
            <Icon name="chevronRight" className="h-3 w-3 transition-transform group-open/think:rotate-90" />
            {thinkingNow ? 'Thinking' : 'Thought it through'}
          </summary>
          <p className="mt-1.5 text-xs whitespace-pre-wrap text-fg-2 italic">{status.thinking}</p>
        </details>
      )}
      {status.streaming && <Markdown text={status.streaming} />}
      {toolOutput && (
        <pre
          data-testid="tool-output"
          className="max-h-48 overflow-auto rounded-sm border border-line-subtle bg-bg px-2.5 py-2 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-fg-2"
        >
          {toolOutput}
        </pre>
      )}
      {working(status) && (
        <div role="status" className="flex items-center gap-2.5 text-xs whitespace-nowrap text-fg-2">
          {/* Blocked on the person, not busy: no spinner. */}
          {status.state === 'awaiting_permission' ? <Dot tone="warn" /> : <Spinner />}
          <span className="text-fg-1">{status.state === 'awaiting_permission' ? 'Waiting on you' : 'Working'}</span>
          {clock && (
            <span className="font-mono" data-volatile>
              {clock}
            </span>
          )}
          {status.context && (
            <span className="font-mono" data-volatile title="How much of the agent's context window the session uses">
              · {tokens(status.context.used)} of {tokens(status.context.size)} context
            </span>
          )}
          {/* The composer's round button stops the turn; here only its key is named. */}
          {status.state === 'working' && (
            <span className="ml-auto flex items-center gap-1.5 pointer-coarse:hidden">
              <StopKey onCancel={onCancel} /> to stop
            </span>
          )}
        </div>
      )}
    </div>
  )
}

/** Starts an agent in the thread on one of the person's online devices, which takes it over (ADR 0026). */
function ContinueOn({ thread, act }: { thread: string; act: (action: Promise<unknown>) => void }) {
  const devices = useQuery({ queryKey: keys.devices, queryFn: api.devices })
  const [device, setDevice] = useState('')
  const [agent, setAgent] = useState('')
  const online = (devices.data ?? []).filter((candidate) => candidate.online && candidate.startable.length > 0)
  const chosen = online.find((candidate) => candidate.id === device) ?? online[0]
  if (!chosen) return <p className="text-xs text-fg-2">No agent runs in this thread, and none of your devices is online to continue it on.</p>
  const chosenAgent = chosen.startable.find((candidate) => candidate.id === agent) ?? chosen.startable[0]
  return (
    <span className="flex flex-wrap items-center gap-1.5 text-xs">
      <span className="text-fg-2">Continue on</span>
      <ChipSelect ghost label="device to continue on" icon="laptop" value={chosen.id} onChange={setDevice} options={online.map((candidate) => ({ id: candidate.id, name: candidate.name }))} />
      <span className="text-fg-2">with</span>
      <ChipSelect ghost label="agent to continue with" value={chosenAgent.id} onChange={setAgent} options={chosen.startable.map((candidate) => ({ id: candidate.id, name: candidate.name }))} />
      <Button variant="primary" onClick={() => act(api.continueOn(thread, chosen.id, chosenAgent.id))}>
        Continue there
      </Button>
    </span>
  )
}

/**
 * The thread's composer, docked under its conversation (ADR 0044): what's
 * queued, what others here are writing, the draft everyone here sees, and
 * the agent's chips. It steers or queues while a turn runs.
 */
export function AgentDock({
  thread,
  status,
  preset,
  onPreset,
  draft,
  onDraft,
  people,
}: {
  thread: string
  status: AgentStatus | null
  preset: Preset
  onPreset: (preset: Preset) => void
  draft: string
  onDraft: (draft: string) => void
  /** Everyone else here, whose machines may run an agent in the thread. */
  people: Presence[]
}) {
  const [chosenAgent, setAgent] = useState<string>()
  const choices = useQuery({ queryKey: keys.agents, queryFn: api.agents, enabled: !hosted() })
  const agents = choices.data?.agents ?? []
  const agent = chosenAgent ?? agents[0]?.id ?? ''
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
  const narrow = typeof window !== 'undefined' && window.matchMedia?.('(max-width: 767px)').matches
  useEffect(() => {
    if (agentName && modes) rememberModes(agentName, modes)
  }, [agentName, modes])
  const turning = working(status)
  useActions([
    ...(live ? [{ id: 'stop', label: `Stop ${status.agent}`, run: () => act(api.stopAgent(thread)) }] : []),
    ...(turning ? [{ id: 'cancel', label: 'Cancel the turn', run: () => act(api.cancelTurn(thread)) }] : []),
    ...(status?.state === 'awaiting_input' ? [{ id: 'approve', label: 'Approve the plan', run: () => act(api.prompt(thread, APPROVED)) }] : []),
  ])
  const sendable = live && draft.trim() !== '' && !(attached.length > 0 && !ready(status))
  const startable = !live && !elsewhere && !hosted() && agent !== ''
  // Idle, the box starts an agent here with the draft as its first prompt, as New thread does.
  const start = () => {
    if (startable) act(api.startAgent(agent, { thread }, { preset, prompt: draft.trim() ? draft : null }), () => onDraft(''))
  }
  const send = () => {
    if (startable) return start()
    if (!live || !sendable) return
    act(api.prompt(thread, draft, false, attached), () => {
      setAttached([])
      onDraft('')
    })
  }

  const statusLine = live ? (
    <span data-testid="agent-state" className="px-1 text-xs text-fg-2">
      {status.agent} {(status.questions?.length ?? 0) > 0 && status.state === 'awaiting_permission' ? 'is asking you a question' : STATE_PHRASE[status.state]}
    </span>
  ) : elsewhere?.agent ? (
    <span data-testid="agent-state" className="px-1 text-xs text-fg-2">
      {elsewhere.user}&apos;s {elsewhere.agent.name} {STATE_PHRASE[elsewhere.agent.state]}
    </span>
  ) : null

  const above = (
    <>
      {!hosted() && <PromptQueue thread={thread} />}
      {elsewhere?.agent && !live && (
        <div className="border-b border-line-subtle px-4 py-2 text-xs text-fg-2">
          {elsewhere.agent.streaming && <Markdown text={elsewhere.agent.streaming} className="mb-1 text-fg-1" />}
          Your draft goes along with {elsewhere.user}&apos;s next prompt.
        </div>
      )}
      {people.some((person) => person.draft.trim()) && (
        <ul data-testid="drafts" aria-label="what others here are writing" className="flex flex-col gap-1 border-b border-line-subtle px-4 py-2 text-xs">
          {people
            .filter((person) => person.draft.trim())
            .map((person) => (
              <li key={person.device}>
                <span className="font-mono text-2xs text-fg-2">{person.user} is drafting:</span> {person.draft}
              </li>
            ))}
        </ul>
      )}
    </>
  )

  const below = attached.length > 0 && (
    <ul aria-label="attached files" className="flex flex-wrap gap-1.5 px-4 pb-2 text-xs">
      {attached.map((file, index) => (
        <li key={`${file.name}-${index}`} className="flex items-center gap-1.5 rounded-sm border border-line bg-surface-1 px-2 py-1">
          <Icon name={file.mime.startsWith('image/') ? 'image' : 'file'} className="h-3.5 w-3.5 text-fg-2" />
          {file.name}
          <button type="button" aria-label={`remove ${file.name}`} className="text-fg-2 hover:text-fg-1" onClick={() => setAttached(attached.filter((_, kept) => kept !== index))}>
            <Icon name="x" className="h-3 w-3" />
          </button>
        </li>
      ))}
    </ul>
  )

  const tools = (
    <>
      {live && !hosted() && (
        <label
          className={cx(GHOST_CHIP, 'w-7 cursor-pointer justify-center px-0 text-fg-2')}
          title={status.takes_images ? 'Attach images or files' : `Attach text files; ${status.agent} takes no images`}
        >
          <Icon name="attach" className="h-4 w-4" />
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
      )}
      {!live && !elsewhere && !hosted() && (
        <ChipSelect
          ghost
          label="agent to start"
          value={agent}
          onChange={setAgent}
          options={agents.map((choice) => ({ id: choice.id, name: choice.name, description: choice.description }))}
        />
      )}
      {!live && !elsewhere && hosted() && <ContinueOn thread={thread} act={act} />}
      {live && status.modes && (
        <ChipSelect
          ghost
          label="mode"
          prefix="Mode"
          value={status.modes.current}
          onChange={(mode) => act(api.setMode(thread, mode))}
          options={status.modes.available.map((mode) => ({ id: mode.id, name: mode.name, description: mode.description }))}
        />
      )}
      {live &&
        status.settings.map((setting) => (
          <ChipSelect
            ghost
            key={setting.id}
            label={setting.name.toLowerCase()}
            value={setting.current}
            onChange={(value) => act(api.setSetting(thread, setting.id, value))}
            options={setting.choices}
          />
        ))}
      {(live || !hosted()) && (
        <ChipSelect
          ghost
          label="permissions"
          icon="lock"
          value={preset}
          onChange={(chosen) => {
            const picked = chosen as Preset
            void api.setPreset(thread, picked).then(() => onPreset(picked))
          }}
          options={PRESETS.map((choice) => ({ id: choice.id, name: choice.label, description: choice.says }))}
        />
      )}
      {live && (
        <Button variant="ghost" size="sm" aria-label="Stop agent" title="Stops the agent; the thread keeps everything it did" onClick={() => act(api.stopAgent(thread))}>
          Stop agent
        </Button>
      )}
    </>
  )

  const startName = agents.find((choice) => choice.id === agent)?.name ?? 'the agent'
  const actions = startable ? (
    <RoundButton
      label="Start agent"
      icon="arrowUp"
      onClick={start}
      title={`Starts ${startName} here${draft.trim() ? ' with your draft' : ''} (${formatKeys('mod+⏎')})`}
    />
  ) : (
    live && (
      <>
        {turning && draft.trim() && (
          <Button variant="ghost" onClick={() => act(api.steer(thread, draft), () => onDraft(''))} title="Interrupts the agent's turn to tell it your draft">
            Steer now
          </Button>
        )}
        {turning && !draft.trim() ? (
          <RoundButton quiet label="Cancel turn" title="Stop the turn (Esc)" icon="stop" onClick={() => act(api.cancelTurn(thread))} />
        ) : (
          <RoundButton
            label={ready(status) ? 'Send' : 'Queue'}
            icon="arrowUp"
            disabled={!sendable}
            onClick={send}
            title={
              ready(status)
                ? `Sends your draft together with everyone else's (${formatKeys('mod+⏎')})`
                : attached.length > 0
                  ? 'Attached files go with a prompt the agent takes now'
                  : `Queues your draft for when the agent is ready (${formatKeys('mod+⏎')})`
            }
          />
        )}
      </>
    )
  )

  return (
    <section aria-label="agent at work" className="flex flex-col gap-2">
      <ComposerBox
        id={`draft-${thread}`}
        label="draft"
        rows={narrow ? 1 : 2}
        value={draft}
        onChange={onDraft}
        onSend={send}
        waiting={status?.state === 'awaiting_input' || status?.state === 'awaiting_permission'}
        placeholder={
          !live
            ? startable
              ? 'Ask for the next step; sending starts the agent'
              : 'Draft the next prompt; everyone here sees it'
            : status.state === 'awaiting_input'
              ? 'Or say what to change in the plan'
              : (status.questions?.length ?? 0) > 0
                ? 'Answer above, or steer it'
                : turning
                  ? narrow
                    ? 'Steer, or queue a prompt'
                    : 'Steer it now, or queue a prompt for when this turn ends'
                : `Ask ${status.agent}, or attach files for context`
        }
        above={above}
        below={below}
        tools={tools}
        actions={actions}
      />
      {statusLine}
      {preset === 'full_access' && (
        <p className="px-1 text-xs text-fg-2">
          <Icon name="alert" className="mr-1 inline h-3 w-3 text-warn" />
          Full access: nothing asks; the agent&apos;s own sandbox is all that limits it.
        </p>
      )}
      {(status?.error || error) && <p className="px-1 text-sm text-danger">{status?.error ?? error}</p>}
    </section>
  )
}
