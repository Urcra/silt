import { Link } from '@tanstack/react-router'
import { type ReactNode, useState } from 'react'
import { useAgentStatus } from '../agentStatus'
import { api } from '../api'
import type { AskView } from '../api.generated'
import { type AskCall, askFor, useAsks } from '../asks'
import { useNow } from '../now'
import { message } from '../query'
import { type ToolStep, duration } from '../transcript'
import { Button, Icon, Spinner, Tag, cx, toast } from '../ui'
import { AgentMark } from './AgentMark'
import { Markdown } from './Markdown'

const STATE: Record<AskView['state'], { label: string; tone: 'ok' | 'danger' | 'neutral' | 'accent' | 'warn' }> = {
  running: { label: 'running', tone: 'accent' },
  done: { label: 'done', tone: 'ok' },
  failed: { label: 'failed', tone: 'danger' },
  stopped: { label: 'stopped', tone: 'neutral' },
}

/** Where the asked agent works and what it may do, and its thread. */
function WhereLine({ ask }: { ask: AskView }) {
  const what =
    ask.mode === 'review'
      ? ask.state === 'running'
        ? 'Review: only its report comes back'
        : 'Review'
      : ask.checkout === 'own'
        ? 'May edit its own checkout'
        : 'May edit your working tree'
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-fg-2">
      <span className="inline-flex items-center gap-1.5">
        <Icon name={ask.mode === 'review' ? 'eye' : 'edit'} className="h-3 w-3" />
        {what}
      </span>
      <span className="inline-flex items-center gap-1.5">
        <Icon name={ask.checkout === 'own' ? 'fork' : 'folder'} className="h-3 w-3" />
        {ask.checkout === 'own' ? 'Forked with your uncommitted edits' : 'In your working tree'}
      </span>
      <Link
        to="/t/$id"
        params={{ id: ask.asked }}
        className="ml-auto inline-flex items-center gap-1 text-fg-1 hover:text-accent-fg max-md:min-h-11"
      >
        Open its thread
        <Icon name="external" className="h-3 w-3" />
      </Link>
    </div>
  )
}

/** An own checkout's changes, waiting to be taken into the thread that asked. */
function TakeRow({ ask }: { ask: AskView }) {
  const [taking, setTaking] = useState(false)
  if (ask.checkout !== 'own' || ask.mode !== 'work' || ask.state === 'running') return null
  return (
    <div className="flex flex-wrap items-center gap-2 rounded-sm border border-line bg-bg px-2.5 py-2">
      <span className="min-w-0 flex-1 text-fg-2">
        {ask.taken ? 'Its changes were taken into this thread.' : 'Its changes are in its own checkout until they’re taken.'}
      </span>
      <Link to="/t/$id" params={{ id: ask.asked }} className="text-fg-1 hover:text-accent-fg max-md:min-h-11">
        Show changes
      </Link>
      {!ask.taken && (
        <Button
          variant="primary"
          size="sm"
          icon="merge"
          disabled={taking}
          onClick={() => {
            setTaking(true)
            api
              .askTake(ask.id)
              .then((said) => toast(said, 'ok'))
              .catch((failure: unknown) => toast(message(failure), 'danger'))
              .finally(() => setTaking(false))
          }}
        >
          Take into this thread
        </Button>
      )}
    </div>
  )
}

/**
 * A call that asked another agent (ADR 0120): one card led by the asked agent's mark and model, opening to where it
 * works, what it was asked, what it did in its thread (`transcript`), and its report. Its mark stays put as it runs, as a
 * subagent's kind does; its tag says how it stands.
 */
export function AskCard({
  step,
  thread,
  asked,
  transcript,
}: {
  step: ToolStep
  thread: string
  asked: AskCall
  transcript: (ask: AskView) => ReactNode
}) {
  const asks = useAsks()
  const call = step.call!
  const ask = askFor(asks, thread, call)
  const status = useAgentStatus(thread)
  // Asking counts as a command, so under a stricter preset the call waits on the person first.
  const waiting = !ask && Boolean(call.tool && status?.permissions.some((asked) => asked.tool_call === call.tool!.id))
  const [open, setOpen] = useState(false)
  const running = ask?.state === 'running'
  const now = useNow(running ? 1000 : 0)
  // Before the ask is known, or for an ask Silt refused: what the call itself says.
  const refused = !ask && step.result?.outcome?.failed
  const shown = ask
    ? STATE[ask.state]
    : waiting
      ? { label: 'asks you', tone: 'warn' as const }
      : refused
        ? { label: 'not asked', tone: 'neutral' as const }
        : step.result
          ? { label: 'done', tone: 'ok' as const }
          : STATE.running
  const took = ask ? duration(ask.started_ms, ask.ended_ms ?? now) : null
  const agent = ask?.agent ?? asked.agent ?? 'agent'
  const name = ask?.agent_name ?? asked.agent ?? 'Another agent'
  const title = ask?.title ?? asked.title ?? asked.prompt.split('\n')[0]
  const why = ask?.why ?? (refused ? step.result?.text.trim() : undefined)
  return (
    <div
      id={call.tool ? `task-${call.tool.id}` : undefined}
      data-testid="ask"
      className="overflow-hidden rounded-md border border-line bg-elevated text-xs"
    >
      <div className={cx('flex items-center', open && 'border-b border-line-subtle')}>
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          title={asked.prompt}
          className="group/row flex w-full min-w-0 flex-1 items-center gap-2 px-3 py-1.5 text-left max-md:min-h-11"
        >
          <Icon name="chevronRight" className={cx('h-3 w-3 shrink-0 text-fg-2 transition-transform', open && 'rotate-90')} />
          <span className="text-fg-1" title={name}>
            <AgentMark agent={agent} name={name} labelled className="h-3.5 w-3.5" />
          </span>
          {ask && <span className="shrink-0 text-fg-2 max-md:hidden">{[ask.model, ask.mode].filter(Boolean).join(' · ')}</span>}
          <span className="min-w-0 flex-1 truncate text-fg-1 group-hover/row:text-accent-fg">{title}</span>
          {took && (
            <span className="shrink-0 font-mono text-fg-3 tabular-nums" data-volatile={running || undefined}>
              {took}
            </span>
          )}
          <Tag tone={shown.tone} className="shrink-0">
            {shown.label}
          </Tag>
        </button>
        {running && ask && (
          <Button
            variant="ghost"
            size="sm"
            icon="stop"
            className="mr-2"
            title={`Stop ${name}'s turn; ${ask.caller_agent} is told`}
            onClick={() => void api.askStop(ask.id).catch((failure: unknown) => toast(message(failure), 'danger'))}
          >
            Stop
          </Button>
        )}
      </div>
      {open && (
        <div className="flex flex-col gap-2 px-3 py-2">
          {ask && <WhereLine ask={ask} />}
          <div className="flex flex-col gap-1">
            <span className="text-2xs text-fg-2">Asked</span>
            <p className="border-l-2 border-line pl-3 text-sm whitespace-pre-wrap text-fg-1">{asked.prompt}</p>
          </div>
          {ask && transcript(ask)}
          {why && <p className="text-fg-2">{why}</p>}
          {ask?.report && (
            <div className="flex flex-col gap-1">
              <span className="text-2xs text-fg-2">Reported back</span>
              <div className="border-l-2 border-line pl-3 text-sm text-fg-1">
                <Markdown text={ask.report} />
              </div>
            </div>
          )}
          {ask && <TakeRow ask={ask} />}
        </div>
      )}
    </div>
  )
}

/** Above the composer: the asks of this thread still running, each a link to its card, with Stop. */
export function AskStrip({ thread }: { thread: string }) {
  const asks = useAsks().filter((ask) => ask.caller === thread && ask.state === 'running')
  const now = useNow(asks.length > 0 ? 1000 : 0)
  if (asks.length === 0) return null
  return (
    <section
      aria-label="asks still running"
      data-testid="running-asks"
      className="flex flex-col gap-1 rounded-md border border-line bg-elevated px-3 py-1.5 text-xs"
    >
      <span className="text-2xs font-medium text-fg-2">Still running beside the agent</span>
      {asks.map((ask) => (
        <div key={ask.id} className="flex min-w-0 items-center gap-2">
          <Link
            to="/t/$id"
            params={{ id: ask.asked }}
            className="flex min-w-0 flex-1 items-center gap-2 text-fg-1 hover:text-accent-fg max-md:min-h-11"
          >
            <Spinner />
            <span className="shrink-0 text-fg-2">Asked</span>
            <span className="text-fg-1" title={ask.agent_name}>
              <AgentMark agent={ask.agent} name={ask.agent_name} labelled className="h-3.5 w-3.5" />
            </span>
            <span className="min-w-0 flex-1 truncate">{ask.title}</span>
            <span className="shrink-0 font-mono text-fg-3 tabular-nums" data-volatile>
              {duration(ask.started_ms, now) ?? '0s'}
            </span>
          </Link>
          <Button
            variant="ghost"
            size="sm"
            icon="stop"
            aria-label={`Stop ${ask.agent_name}`}
            onClick={() => void api.askStop(ask.id).catch((failure: unknown) => toast(message(failure), 'danger'))}
          >
            Stop
          </Button>
        </div>
      ))}
    </section>
  )
}

/** The note atop a thread another agent asked for: who waits on it, and where its report goes. */
export function AskedNote({ ask }: { ask: AskView }) {
  const waiting = ask.state === 'running'
  return (
    <div
      role="status"
      data-testid="asked-note"
      className="flex items-start gap-2 rounded-md border border-line bg-info-tint px-3 py-2 text-xs text-fg-1"
    >
      <Icon name="info" className="mt-0.5 h-3.5 w-3.5 shrink-0 text-info" />
      <span className="min-w-0 flex-1">
        {waiting
          ? `${ask.caller_agent} is waiting for this thread’s report. When ${ask.agent_name} ends its turn, its last message goes back to ${ask.caller_agent}. What you say here reaches ${ask.agent_name}, not ${ask.caller_agent}.`
          : `${ask.caller_agent} asked for this thread. ${ask.agent_name}’s last message went back to it as its report.`}
      </span>
      <Link to="/t/$id" params={{ id: ask.caller }} className="shrink-0 text-fg-1 underline decoration-line underline-offset-2">
        Open {ask.caller_agent}’s thread
      </Link>
    </div>
  )
}
