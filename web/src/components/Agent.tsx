import { useQuery } from '@tanstack/react-query'
import { type Dispatch, type ReactNode, type SetStateAction, useEffect, useRef, useState } from 'react'
import {
  api,
  type AgentStatus,
  type PermissionAsked,
  type Presence,
  type Preset,
  type PromptAttachment,
  type SignInAsked,
} from '../api'
import { APPROVED, DEFAULT_PRESETS, PRESETS, STATE_PHRASE, permissionModes, running, startSettings } from '../agentState'
import { useNow } from '../now'
import { tokens } from '../transcript'
import { keys, message } from '../query'
import { hosted } from '../served'
import { UsageRing } from './UsageRing'
import { formatKeys, useActions, useShortcut } from '../shortcuts'
import { Button, Dot, Icon, Input, Kbd, Spinner, buttonClass, toast } from '../ui'
import { AgentSettings } from './AgentSettings'
import { agentRefused, modeRefused, presetRefused, workspaceOf } from '../rules'
import { ATTACH, useAttachments } from '../attachments'
import { AttachedFiles } from './Attachments'
import { ChipSelect } from './ChipSelect'
import { ComposerBox, RoundButton } from './Composer'
import { Markdown, StreamingMarkdown } from './Markdown'
import { PromptQueue } from './Queue'
import { dropSending } from '../sending'

export function AgentPicker({
  value,
  onChange,
  className,
}: {
  value: string
  onChange: (agent: string) => void
  className?: string
}) {
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

function statePhrase(status: AgentStatus): string {
  if (status.sign_in) return `${status.agent} needs you to sign in`
  const asking = (status.questions?.length ?? 0) > 0 && status.state === 'awaiting_permission'
  return `${status.agent} ${asking ? 'is asking you a question' : STATE_PHRASE[status.state]}`
}

/** A permission the agent asks for, answered in place: ⏎ allows, Esc dismisses (ADR 0022). */
export function PermissionCard({
  thread,
  asked,
  agent,
  first,
}: {
  thread: string
  asked: PermissionAsked
  agent: string
  first: boolean
}) {
  const [error, setError] = useState<string>()
  const answer = (option: string | null) => {
    setError(undefined)
    api.answer(thread, asked.id, option).catch((failure: unknown) => setError(message(failure)))
  }
  const plan = asked.plan ? planChoices(asked.options) : null
  const [then, setThen] = useState(plan?.approve[0]?.[0])
  const allow = plan?.approve.length ? then : asked.options[0]?.[0]
  useShortcut('enter', 'Allow what the agent asks', () => {
    if (first && allow && !document.querySelector('dialog[open]')) answer(allow)
  })
  useShortcut('escape', 'Answer neither and stop the turn', () => {
    if (first && !document.querySelector('dialog[open]')) answer(null)
  })
  return (
    <div
      role="group"
      aria-label={asked.plan ? 'plan' : 'permission'}
      className="flex flex-col gap-2.5 rounded-md border border-warn bg-elevated px-3.5 py-3"
    >
      <span className="flex min-w-0 items-center gap-2 text-xs text-fg-2">
        <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn" />
        {asked.plan ? `${agent}'s plan, to approve before it changes anything` : `${agent} asks to run`}
      </span>
      {/* Whole, with its own line breaks: what's approved must be readable to its end. */}
      {!asked.plan && (
        <pre className="max-h-64 overflow-y-auto rounded-sm border border-line bg-bg px-2.5 py-2 font-mono text-xs whitespace-pre-wrap text-fg-1 [overflow-wrap:anywhere]">
          {asked.title}
        </pre>
      )}
      {/* In the page's own flow, not a box of its own to scroll: the plan is what's being read. */}
      {asked.plan && (
        <div className="rounded-sm border border-line-subtle bg-bg px-3.5 py-2.5 text-sm">
          <Markdown text={asked.plan} />
        </div>
      )}
      {plan && then && plan.approve.length > 0 ? (
        <PlanChoices choices={plan} then={then} onThen={setThen} answer={answer} first={first} />
      ) : (
        <span className="flex flex-wrap gap-1.5">
          {asked.options.map(([option, label], index) => (
            <Button key={option} variant={index === 0 ? 'primary' : 'secondary'} onClick={() => answer(option)}>
              {label}
              {index === 0 && first && <Kbd onAccent>⏎</Kbd>}
            </Button>
          ))}
          <Button
            variant="ghost"
            title="Answers neither: the agent drops what it asked and ends its turn"
            onClick={() => answer(null)}
          >
            Stop the turn {first && <Kbd>Esc</Kbd>}
          </Button>
        </span>
      )}
      {error && <span className="text-xs text-danger">{error}</span>}
    </div>
  )
}

/** The agent's own sign-in expired: signed in from here, the sign-in running on the agent's machine, and the agent going on
 * once it's done (ADR 0056). Only a link the sign-in prints, on https, is offered to open. */
export function SignInCard({ thread, asked, agent }: { thread: string; asked: SignInAsked; agent: string }) {
  const [error, setError] = useState<string>()
  const [code, setCode] = useState('')
  const act = (action: Promise<unknown>) => {
    setError(undefined)
    action.catch((failure: unknown) => setError(message(failure)))
  }
  const running = asked.running
  const byAgent = running && asked.methods.find((method) => method.id === running.method)?.by_agent
  const link = running?.link?.startsWith('https://') ? running.link : null
  const send = () => {
    if (!code.trim()) return
    act(api.signInCode(thread, code.trim()).then(() => setCode('')))
  }
  return (
    <div
      role="group"
      aria-label="sign in"
      className="flex flex-col gap-2.5 rounded-md border border-warn bg-elevated px-3.5 py-3"
    >
      <span className="flex min-w-0 items-center gap-2 text-xs text-fg-2">
        <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn" />
        {agent} needs you to sign in
      </span>
      {!running ? (
        <>
          <p className="text-sm text-fg-1">
            {asked.methods.length > 0
              ? `Its own sign-in has expired, or was never made. Signing in runs where ${agent} runs, and then it goes on with what you asked.`
              : `It lists no way to sign in that Silt can follow; sign it in on the machine it runs on, then start it again.`}
          </p>
          <span className="flex flex-wrap gap-1.5">
            {asked.methods.map((method, index) => (
              <Button
                key={method.id}
                variant={index === 0 ? 'primary' : 'secondary'}
                title={method.description ?? undefined}
                onClick={() => act(api.signIn(thread, method.id))}
              >
                {method.name}
              </Button>
            ))}
          </span>
        </>
      ) : byAgent ? (
        <p role="status" className="flex items-center gap-2 text-sm text-fg-1">
          <Spinner /> {agent} is signing in on its machine
        </p>
      ) : (
        <>
          {link ? (
            <div className="flex flex-col gap-1.5">
              <span className="text-sm text-fg-1">
                {running.code ? 'Open this page and enter the code:' : 'Open this page and sign in:'}
              </span>
              <span className="flex flex-wrap items-center gap-1.5">
                <a
                  href={link}
                  target="_blank"
                  rel="noopener noreferrer"
                  className={buttonClass(running.code || running.asks_code ? 'primary' : 'secondary')}
                >
                  Open {new URL(link).host}
                </a>
                <Button
                  variant="ghost"
                  onClick={() => {
                    void navigator.clipboard?.writeText(link).catch(() => undefined)
                    toast('Link copied', 'ok')
                  }}
                >
                  Copy link
                </Button>
              </span>
              {running.code && (
                <code
                  data-testid="sign-in-code"
                  className="self-start rounded-sm border border-line bg-bg px-2.5 py-1 font-mono text-lg tracking-widest text-fg-1 select-all"
                >
                  {running.code}
                </code>
              )}
            </div>
          ) : (
            <p role="status" className="flex items-center gap-2 text-sm text-fg-1">
              <Spinner /> Starting the sign-in
            </p>
          )}
          {running.asks_code && (
            <form
              className="flex flex-wrap items-center gap-1.5"
              onSubmit={(event) => {
                event.preventDefault()
                send()
              }}
            >
              <Input
                aria-label="code from the sign-in page"
                placeholder="Paste the code the page shows"
                autoComplete="one-time-code"
                spellCheck={false}
                value={code}
                onChange={(event) => setCode(event.target.value)}
                className="min-w-0 flex-1 font-mono"
              />
              <Button type="submit" variant="primary" disabled={!code.trim()}>
                Sign in
              </Button>
            </form>
          )}
          {link && !running.asks_code && (
            <p role="status" className="flex items-center gap-2 text-xs text-fg-2">
              <Spinner /> {running.code_sent ? 'Checking the code' : 'Waiting for the sign-in to finish'}
            </p>
          )}
          <span className="flex flex-wrap items-center gap-1.5">
            {running.tail && (
              <details className="min-w-0 flex-1 text-xs text-fg-2">
                <summary className="cursor-pointer">What the sign-in shows</summary>
                <pre className="mt-1.5 max-h-48 overflow-y-auto rounded-sm border border-line bg-bg px-2.5 py-2 font-mono whitespace-pre-wrap [overflow-wrap:anywhere]">
                  {running.tail}
                </pre>
              </details>
            )}
            <Button variant="ghost" className="ml-auto" onClick={() => act(api.cancelSignIn(thread))}>
              Cancel
            </Button>
          </span>
        </>
      )}
      {asked.failed && !running && <span className="text-xs text-danger">{asked.failed}</span>}
      {error && <span className="text-xs text-danger">{error}</span>}
    </div>
  )
}

/** How approving a plan leaves the thread, in Silt's presets, by the option each is answered with (Claude's ExitPlanMode). */
const PLAN_PRESETS: [option: string, preset: string][] = [
  ['exit-plan-accept-edits', 'Ask before commands'],
  ['exit-plan-default', 'Ask for everything'],
  ['exit-plan-auto', 'Full access'],
]
const KEEP_PLANNING = 'reject'

function planChoices(options: [string, string][]) {
  const ids = new Set(options.map(([id]) => id))
  const approve = PLAN_PRESETS.filter(([option]) => ids.has(option))
  const known = new Set([...approve.map(([option]) => option), KEEP_PLANNING])
  return { approve, keep: ids.has(KEEP_PLANNING), others: options.filter(([id]) => !known.has(id)) }
}

/** A plan's answers as ADR 0044 has them: approve, then the preset to go on with; or keep planning. The agent's own other answers stay at hand. */
function PlanChoices({
  choices: { approve, keep, others },
  then,
  onThen,
  answer,
  first,
}: {
  choices: ReturnType<typeof planChoices>
  then: string
  onThen: (option: string) => void
  answer: (option: string | null) => void
  first: boolean
}) {
  return (
    <div className="flex flex-col gap-2">
      <span className="flex flex-wrap items-center gap-1.5">
        <Button variant="primary" onClick={() => answer(then)}>
          Approve plan {first && <Kbd onAccent>⏎</Kbd>}
        </Button>
        <ChipSelect
          label="after approving, the permissions"
          prefix="then"
          icon="lock"
          value={then}
          onChange={onThen}
          options={approve.map(([option, preset]) => ({ id: option, name: preset }))}
        />
        {keep && (
          <Button
            variant="ghost"
            onClick={() => answer(KEEP_PLANNING)}
            title="The agent goes on planning; say what to change in the composer"
          >
            Keep planning
          </Button>
        )}
      </span>
      {others.length > 0 && (
        <details className="text-xs text-fg-2">
          <summary className="cursor-pointer">Other choices</summary>
          <span className="mt-1.5 flex flex-wrap gap-1.5">
            {others.map(([option, label]) => (
              <Button key={option} size="sm" variant="secondary" onClick={() => answer(option)}>
                {label}
              </Button>
            ))}
          </span>
        </details>
      )}
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
    <section
      role="group"
      aria-label="plan"
      className="flex flex-col gap-2.5 rounded-md border border-warn bg-elevated px-3.5 py-3"
    >
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

/** What the agent is doing now, before it is recorded: its thinking, its words, and how long it's been at it. The running
 * tool's output stays in its step, a click away, rather than streaming into the thread (ADR 0049). */
export function AgentTail({
  status,
  thoughtShown,
  onCancel,
}: {
  status: AgentStatus
  thoughtShown: boolean
  onCancel: () => void
}) {
  const clock = useTurnClock(status)
  if (!working(status) && !status.streaming) return null
  // The daemon records the thought once the agent says something, runs a tool or ends its turn, so one unrecorded is under way.
  const thinkingNow = status.state === 'working' && !status.streaming
  return (
    <div className="flex flex-col gap-3" aria-live="polite">
      {status.thinking && !thoughtShown && (
        // Open while it's what the agent does; folded once it answers, runs a tool or asks.
        <details key={thinkingNow ? 'now' : 'done'} open={thinkingNow} className="group/think border-l-2 border-line pl-3">
          <summary className="flex cursor-pointer list-none items-center gap-1.5 text-xs text-fg-2">
            <Icon name="chevronRight" className="h-3 w-3 transition-transform group-open/think:rotate-90" />
            {thinkingNow ? 'Thinking' : 'Thought'}
          </summary>
          <p className="mt-1.5 text-xs whitespace-pre-wrap text-fg-2 italic">{status.thinking}</p>
        </details>
      )}
      {status.streaming && <StreamingMarkdown text={status.streaming} />}
      {working(status) && (
        <div role="status" className="flex items-center gap-2.5 text-xs whitespace-nowrap text-fg-2">
          {/* Blocked on the person, not busy: no spinner. */}
          {status.state === 'awaiting_permission' ? <Dot tone="warn" /> : <Spinner />}
          <span data-testid="agent-state" className="min-w-0 truncate text-fg-1">
            {statePhrase(status)}
          </span>
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
  if (!chosen)
    return (
      <p className="text-xs text-fg-2">No agent runs in this thread, and none of your devices is online to continue it on.</p>
    )
  const chosenAgent = chosen.startable.find((candidate) => candidate.id === agent) ?? chosen.startable[0]
  return (
    <span className="flex flex-wrap items-center gap-1.5 text-xs">
      <span className="text-fg-2">Continue on</span>
      <ChipSelect
        ghost
        label="device to continue on"
        icon="laptop"
        value={chosen.id}
        onChange={setDevice}
        options={online.map((candidate) => ({ id: candidate.id, name: candidate.name }))}
      />
      <span className="text-fg-2">with</span>
      <ChipSelect
        ghost
        label="agent to continue with"
        value={chosenAgent.id}
        onChange={setAgent}
        options={chosen.startable.map((candidate) => ({ id: candidate.id, name: candidate.name }))}
      />
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
  heard,
  preset,
  onPreset,
  draft,
  onDraft,
  onSent,
  onSending,
  people,
  top,
}: {
  thread: string
  status: AgentStatus | null
  /** Whether the page has heard how the agent stands; until then a null `status` doesn't mean none runs. */
  heard: boolean
  preset: Preset
  onPreset: (preset: Preset) => void
  draft: string
  onDraft: Dispatch<SetStateAction<string>>
  /** The agent took a prompt, so the page goes to where its turn will show. */
  onSent?: () => void
  /** A prompt goes straight to the agent: the page shows it at once, until the history has it. */
  onSending?: (text: string, files: string[]) => number
  /** Everyone else here, whose machines may run an agent in the thread. */
  people: Presence[]
  /** On the composer's top edge. */
  top?: ReactNode
}) {
  const [chosenAgent, setAgent] = useState<string>()
  const choices = useQuery({ queryKey: keys.agents, queryFn: api.agents, enabled: !hosted() })
  const agents = choices.data?.agents ?? []
  // The thread's workspace, whose rules grey out what the daemon would refuse (ADR 0065).
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads, enabled: !hosted() })
  const workspaces = useQuery({ queryKey: keys.workspaces, queryFn: api.workspaces, enabled: !hosted() })
  const summary = threads.data?.find((candidate) => candidate.id === thread)
  const workspace = summary ? workspaceOf(workspaces.data, summary.repo, summary.workspace) : undefined
  const agent =
    (chosenAgent && !agentRefused(workspace, chosenAgent) ? chosenAgent : undefined) ??
    agents.find((choice) => !agentRefused(workspace, choice.id))?.id ??
    ''
  const [error, setError] = useState<string>()
  const act = (action: Promise<unknown>, after?: () => void) => {
    setError(undefined)
    action.then(after, (failure: unknown) => setError(message(failure)))
  }
  const live = status !== null && running(status.state)
  const elsewhere = people.find((person) => person.agent)
  const narrow = typeof window !== 'undefined' && window.matchMedia?.('(max-width: 767px)').matches
  // Before an agent runs here, its modes and settings as it last ran on this machine, to start it with.
  const seen = agents.find((choice) => choice.id === agent)
  const seenModes = seen?.modes?.available
  // Before it runs, its mode is offered where its modes are its permissions; else the preset says which it starts in.
  const startModes = permissionModes(seenModes) ? seenModes : undefined
  const seenSettings = startSettings(seen?.settings ?? [], seenModes)
  const [mode, setChosenMode] = useState<string>()
  const chosenMode = mode ?? seen?.modes?.current
  const [chosenSettings, setChosenSettings] = useState<Record<string, string>>({})
  const { attached, add, remove, clear, restore } = useAttachments()
  const chooseAgent = (chosen: string) => {
    setAgent(chosen)
    setChosenMode(undefined)
    setChosenSettings({})
  }
  // Another agent takes the thread over: this one stops, keeping all it did, and the other starts here.
  const switchAgent = (chosen: string) => {
    if (live && chosen !== status.agent_id) act(api.stopAgent(thread).then(() => api.startAgent(chosen, { thread }, { preset })))
  }
  const turning = working(status)
  useActions([
    ...(live ? [{ id: 'stop', label: `Stop ${status.agent}`, run: () => act(api.stopAgent(thread)) }] : []),
    ...(turning ? [{ id: 'cancel', label: 'Cancel the turn', run: () => act(api.cancelTurn(thread)) }] : []),
    ...(status?.state === 'awaiting_input'
      ? [{ id: 'approve', label: 'Approve the plan', run: () => act(api.prompt(thread, APPROVED)) }]
      : []),
  ])
  const sendable = live && draft.trim() !== '' && !(attached.length > 0 && !ready(status))
  const startable = !live && !elsewhere && !hosted() && agent !== ''
  // What was written while the request was under way stays to be sent next.
  const sent = (text: string) => () => {
    onDraft((current) => (current === text ? '' : current))
    if (text.trim()) onSent?.()
  }
  // The prompt shows as sent at once, and the box empties; if the agent doesn't take it, both come back.
  const optimistic = (action: (text: string, files: PromptAttachment[]) => Promise<unknown>) => {
    const text = draft
    const files = attached
    const sending = text.trim()
      ? onSending?.(
          text,
          files.map((file) => file.name),
        )
      : undefined
    setError(undefined)
    onDraft('')
    clear()
    if (text.trim()) onSent?.()
    action(text, files).catch((failure: unknown) => {
      if (sending !== undefined) dropSending(thread, [sending])
      onDraft((current) => (current.trim() ? current : text))
      restore(files)
      setError(message(failure))
    })
  }
  // The agent asked to start here, until the page hears how it stands: ⏎ meanwhile mustn't start it again.
  const starting = useRef<Promise<unknown>>(undefined)
  const state = status?.state
  useEffect(() => {
    starting.current = undefined
  }, [state, thread])
  // Idle, the box starts an agent here with the draft as its first prompt, as New thread does.
  const start = () => {
    if (!startable) return
    const settings = seenSettings.map((setting): [string, string] => [setting.id, chosenSettings[setting.id] ?? setting.current])
    const mode = startModes ? (chosenMode ?? null) : null
    optimistic((text, files) => {
      const started = api.startAgent(
        agent,
        { thread },
        { preset, mode, settings, prompt: text.trim() ? text : null, attachments: files },
      )
      starting.current = started
      const failed = () => {
        if (starting.current === started) starting.current = undefined
      }
      started.catch(failed)
      return started
    })
  }
  // The draft goes once the agent has started, which queues it as it does any prompt before the agent is ready.
  const queueAfter = (pending: Promise<unknown>) => {
    if (!draft.trim()) return
    const text = draft
    const files = attached
    setError(undefined)
    onDraft('')
    clear()
    pending
      .then(() => api.prompt(thread, text, false, files))
      .catch((failure: unknown) => {
        onDraft((current) => (current.trim() ? `${current}\n\n${text}` : text))
        restore(files)
        setError(message(failure))
      })
  }
  const steer = () => act(api.steer(thread, draft), sent(draft))
  // ⏎ before the page knows whether an agent runs, or which it may start, goes once it knows, rather than start
  // one that's running already or drop the draft.
  const known = heard && (hosted() || !choices.isPending)
  const held = useRef<{ now: boolean }>(undefined)
  // `now`: mod+⏎, which interrupts a turn under way to say the draft at once rather than queue it.
  const send = (now = false) => {
    if (!known) {
      held.current = { now }
      return
    }
    if (starting.current) return queueAfter(starting.current)
    if (startable) return start()
    if (now && turning && draft.trim()) return steer()
    if (!live || !sendable) return
    // A queued prompt shows in the queue once the daemon has it, not in the history.
    if (!ready(status))
      return act(api.prompt(thread, draft, false, attached), () => {
        clear()
        sent(draft)()
      })
    optimistic((text, files) => api.prompt(thread, text, false, files))
  }
  useEffect(() => {
    if (!known || !held.current) return
    const { now } = held.current
    held.current = undefined
    send(now)
  })

  // While a turn runs, the line above the composer says so, with its clock.
  // The preset picked counts at once, as starting the agent takes it along: waiting for the daemon's answer, a start
  // in between would carry the one before and undo the pick. Refused, it goes back.
  const choosePreset = (picked: Preset) => {
    const before = preset
    onPreset(picked)
    void api.setPreset(thread, picked).catch((failure: unknown) => {
      onPreset(before)
      setError(message(failure))
    })
  }

  // Shift+Tab moves to the agent's next mode, or Silt's next preset for an agent without modes.
  const cycle = () => {
    if (live && status.modes && permissionModes(status.modes.available)) {
      const modes = status.modes.available
      const at = modes.findIndex((mode) => mode.id === status.modes!.current)
      const next = modes[(at + 1) % modes.length]
      if (next) act(api.setMode(thread, next.id))
      return
    }
    if (hosted() && !live) return
    const at = PRESETS.findIndex((choice) => choice.id === preset)
    choosePreset(PRESETS[(at + 1) % PRESETS.length]!.id)
  }

  // Ready, waiting or planned, the page shows it already (the header, the cards); this says it only to a screen reader.
  const statusLine = live ? (
    !working(status) && (
      <span data-testid="agent-state" role="status" className="sr-only">
        {statePhrase(status)}
      </span>
    )
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
        <ul
          data-testid="drafts"
          aria-label="what others here are writing"
          className="flex flex-col gap-1 border-b border-line-subtle px-4 py-2 text-xs"
        >
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

  const below = <AttachedFiles files={attached} onRemove={remove} />
  const attachable = (live || startable) && !hosted()
  const attach = (files: File[], pasted = false) => act(add(files, pasted))

  const tools = (
    <>
      {/* The same row as New thread's: which agent, then its mode and settings. Another agent here takes the thread over. */}
      {(live ? !hosted() && status.agent_id !== '' : !elsewhere && !hosted()) && agents.length > 0 && (
        <ChipSelect
          ghost
          label="agent to start"
          value={live ? status.agent_id : agent}
          onChange={(chosen) => (live ? switchAgent(chosen) : chooseAgent(chosen))}
          options={agents.map((choice) => ({ id: choice.id, name: choice.name, description: choice.description }))}
        />
      )}
      {!live && !elsewhere && !hosted() && choices.isSuccess && agents.length === 0 && (
        <span className="px-2 text-xs text-fg-2" title="No agent CLI is on the daemon's PATH; see New thread for how to add one">
          No agent found
        </span>
      )}
      {!live && !elsewhere && hosted() && <ContinueOn thread={thread} act={act} />}
      {live ? (
        <AgentSettings
          modes={status.modes?.available}
          mode={status.modes?.current}
          onMode={(mode) => act(api.setMode(thread, mode))}
          refusedMode={(mode) => modeRefused(workspace, status.agent_id, mode)}
          settings={status.settings}
          onSetting={(id, value) => act(api.setSetting(thread, id, value))}
          more={
            <Button
              variant="ghost"
              size="sm"
              title="Stops the agent; the thread keeps everything it did"
              onClick={() => act(api.stopAgent(thread))}
            >
              Stop agent
            </Button>
          }
          narrow={narrow}
        />
      ) : (
        startable && (
          <AgentSettings
            modes={startModes}
            mode={chosenMode}
            onMode={setChosenMode}
            settings={seenSettings}
            value={(setting) => chosenSettings[setting.id] ?? setting.current}
            onSetting={(id, value) => setChosenSettings({ ...chosenSettings, [id]: value })}
            narrow={narrow}
          />
        )
      )}
      {/* An agent whose modes are about permissions has them as its one control; the preset follows them (ADR 0048). */}
      {(live || !hosted()) && !(live ? permissionModes(status.modes?.available) : startable && permissionModes(seenModes)) && (
        <ChipSelect
          ghost
          label="permissions"
          icon="lock"
          value={preset}
          onChange={(chosen) => choosePreset(chosen as Preset)}
          options={PRESETS.map((choice) => ({
            id: choice.id,
            name: choice.label,
            description: choice.says,
            refused: presetRefused(workspace, choice.id),
          }))}
        />
      )}
      {(live ? status.agent_id : startable && agent) && <UsageRing agent={live ? status.agent_id : agent} thread={thread} />}
    </>
  )

  const startName = agents.find((choice) => choice.id === agent)?.name ?? 'the agent'
  // ⏎ sends, so the box keeps only stopping the turn; a touch keyboard's ⏎ breaks the line, so there a button sends.
  const actions = turning && (
    <RoundButton quiet label="Cancel turn" title="Stop the turn (Esc)" icon="stop" onClick={() => act(api.cancelTurn(thread))} />
  )
  const touchSend = startable
    ? { label: 'Start agent', title: `Starts ${startName} here${draft.trim() ? ' with your draft' : ''}` }
    : live && draft.trim()
      ? {
          label: ready(status) ? 'Send' : 'Queue',
          disabled: !sendable,
          title: ready(status)
            ? "Sends your draft together with everyone else's"
            : attached.length > 0
              ? 'Attached files go with a prompt the agent takes now'
              : 'Queues your draft for when the agent is ready',
        }
      : undefined
  const enter = startable ? `start ${startName}` : live && !ready(status) ? 'queue' : 'send'
  const note = turning && draft.trim() && (
    <Button
      variant="ghost"
      size="sm"
      onClick={steer}
      title={`Interrupts the agent's turn to tell it your draft (${formatKeys('mod+⏎')})`}
    >
      Steer now
      <span className="font-mono text-2xs text-fg-3">{formatKeys('mod+⏎')}</span>
    </Button>
  )

  return (
    <section aria-label="agent at work" className="flex flex-col gap-2">
      <ComposerBox
        id={`draft-${thread}`}
        label="draft"
        rows={1}
        layout="inline"
        value={draft}
        onChange={onDraft}
        onSend={send}
        waiting={status?.state === 'awaiting_input' || status?.state === 'awaiting_permission'}
        onCycle={cycle}
        onFiles={attachable ? attach : undefined}
        attach={ATTACH}
        commands={(live ? status.commands : undefined) ?? seen?.commands}
        voice={!hosted()}
        suggestion={live && ready(status) ? (status.suggestion ?? undefined) : undefined}
        placeholder={
          !live
            ? startable
              ? 'Ask for the next step; sending starts the agent'
              : 'Draft the next prompt; everyone here sees it'
            : status.state === 'awaiting_input'
              ? 'Or say what to change in the plan'
              : status.sign_in
                ? 'Sign in above; a prompt waits for it'
                : (status.questions?.length ?? 0) > 0
                  ? 'Answer above, or steer it'
                  : turning
                    ? narrow
                      ? 'Steer, or queue a prompt'
                      : 'Steer it now, or queue a prompt for when this turn ends'
                    : `Ask ${status.agent}, or attach files for context`
        }
        top={top}
        above={above}
        below={below}
        tools={tools}
        actions={actions}
        note={note}
        send={touchSend}
        enter={enter}
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
