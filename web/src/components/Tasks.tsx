import { type ReactNode, useContext, useState } from 'react'
import { useNow } from '../now'
import { SessionLive, StoppableTasks, type TaskState, type Tasks, TasksContext, running } from '../tasks'
import { type ToolStep, duration, kindOf, shortCommand, toolOf } from '../transcript'
import { Button, Icon, Spinner, Tag, cx } from '../ui'
import { Markdown } from './Markdown'
import { useWhole } from '../wholeEntry'

/** What a call left running: its task's news, else what its call and result say. */
function stateOf(step: ToolStep, tasks: Tasks): { task: TaskState | null; subagent: boolean; status: string } {
  const call = step.call!
  const task = (call.tool && tasks.byCall.get(call.tool.id)) || null
  const subagent = task?.kind === 'subagent' || (task?.kind !== 'background' && kindOf(call) === 'subagent')
  // A call whose agent gives no news ends with its result.
  const status = task?.status ?? (!step.result ? 'running' : step.result.outcome?.failed ? 'failed' : 'completed')
  return { task, subagent, status }
}

const STATUS: Record<string, { label: string; tone: 'ok' | 'danger' | 'neutral' | 'accent' }> = {
  completed: { label: 'done', tone: 'ok' },
  failed: { label: 'failed', tone: 'danger' },
  stopped: { label: 'stopped', tone: 'neutral' },
  running: { label: 'running', tone: 'accent' },
}

/** What a task is, as a line names it: a subagent's description, or the command left running. */
function titleOf(step: ToolStep, subagent: boolean): string {
  const call = step.call!
  if (subagent) return call.tool?.subject?.split('\n')[0] ?? 'A subagent'
  return call.tool?.detail?.trim() || shortCommand((call.tool?.subject ?? toolOf(call.text).name).split('\n')[0]!)
}

/**
 * A call that started a subagent or left a command running beside the turn: one card that says what it is and how it
 * stands, opening to what the subagent did (`children`) and what came back.
 */
export function TaskCard({ step, children }: { step: ToolStep; children?: ReactNode }) {
  const tasks = useContext(TasksContext)
  const live = useContext(SessionLive)
  const stoppable = useContext(StoppableTasks)
  const call = step.call!
  const { task, subagent, status } = stateOf(step, tasks)
  // Only a task still running counts the seconds: every card in a long thread ticked once a second, for good.
  const now = useNow(status === 'running' ? 1000 : 0)
  const [open, setOpen] = useState(false)
  const result = useWhole(step.result, open)
  const quiet = status === 'running' && !live
  const shown = quiet ? { label: 'no word since', tone: 'neutral' as const } : (STATUS[status] ?? STATUS.running)
  const ended = task?.ended_ms ?? (task ? null : (step.result?.time_ms ?? null))
  const took =
    status === 'running' ? (quiet ? null : duration(call.time_ms, now)) : ended !== null ? duration(call.time_ms, ended) : null
  const started = task?.resumes?.tool ?? call.tool
  const kind = subagent ? started?.detail?.trim() || 'Subagent' : 'In the background'
  const name = subagent ? started?.subagent?.trim() : undefined
  // What came back: a subagent's report, or a background command's launch message and what it said when it ended.
  // A message that resumed a subagent, as Claude's SendMessage, only says it did; the run's report is its news.
  const report = subagent && kindOf(call) === 'subagent' && status !== 'running' ? result?.text.trim() : undefined
  const said = task?.summary?.trim()
  const canStop = status === 'running' && call.tool !== null && stoppable.calls.has(call.tool.id)
  return (
    <div
      id={call.tool ? `task-${call.tool.id}` : undefined}
      data-testid="task"
      className="overflow-hidden rounded-md border border-line bg-elevated text-xs"
    >
      <div className={cx('flex items-center', open && 'border-b border-line-subtle')}>
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          title={call.tool?.subject ?? undefined}
          className="group/row flex w-full min-w-0 flex-1 items-center gap-2 px-3 py-1.5 text-left max-md:min-h-11"
        >
          <Icon name="chevronRight" className={cx('h-3 w-3 shrink-0 text-fg-2 transition-transform', open && 'rotate-90')} />
          {status === 'running' && !quiet ? (
            <Spinner />
          ) : (
            <Icon name={subagent ? 'agent' : 'terminal'} className="h-3.5 w-3.5 shrink-0 text-fg-2" />
          )}
          {name && (
            <span data-testid="task-name" className="max-w-[40%] shrink-0 truncate font-medium text-fg-1">
              {name}
            </span>
          )}
          <span className="shrink-0 text-fg-2">{task?.resumes ? `${kind}, resumed` : kind}</span>
          <span className="min-w-0 flex-1 truncate text-fg-1 group-hover/row:text-accent-fg">{titleOf(step, subagent)}</span>
          {took && (
            <span className="shrink-0 font-mono text-fg-3 tabular-nums" data-volatile={status === 'running' || undefined}>
              {took}
            </span>
          )}
          <Tag tone={shown.tone} className="shrink-0">
            {shown.label}
          </Tag>
        </button>
        {canStop && (
          <Button
            variant="ghost"
            size="sm"
            icon="stop"
            className="mr-2"
            title="End the command and what it started"
            onClick={() => stoppable.stop(call.tool!.id)}
          >
            Stop
          </Button>
        )}
      </div>
      {open && (
        <div className="flex flex-col gap-2 px-3 py-2">
          {quiet && <p className="text-fg-2">Nothing has said how it ended. Its session may have stopped with it running.</p>}
          {!subagent && call.tool?.subject && (
            <pre className="font-mono text-output whitespace-pre-wrap text-fg-1 [overflow-wrap:anywhere]">
              <span className="text-fg-2">$ </span>
              {call.tool.subject}
            </pre>
          )}
          {subagent && call.tool?.subject && call.tool.subject.includes('\n') && (
            <p className="whitespace-pre-wrap text-fg-2">{call.tool.subject}</p>
          )}
          {children}
          {!subagent && result?.text.trim() && (
            <pre className="max-h-40 overflow-auto font-mono text-output whitespace-pre-wrap text-fg-2">{result.text.trim()}</pre>
          )}
          {said && (
            <div className="border-l-2 border-line pl-3 text-sm text-fg-1">
              <Markdown text={said} />
            </div>
          )}
          {report && (
            <div className="flex flex-col gap-1">
              <span className="text-2xs text-fg-2">Reported back</span>
              <div className="border-l-2 border-line pl-3 text-sm text-fg-1">
                <Markdown text={report} />
              </div>
            </div>
          )}
          {!children && subagent && !report && status === 'running' && (
            <p className="text-fg-2">Its steps show here as it takes them.</p>
          )}
        </div>
      )}
    </div>
  )
}

/**
 * Above the composer: the subagents and background commands still running beside the agent, each a link to its card.
 * Only while the session may still run them; a task its session left running and never reported on shows only on its card.
 */
export function RunningStrip({ labels }: { labels: (task: TaskState) => { title: string; name: string | null } }) {
  const tasks = useContext(TasksContext)
  const live = useContext(SessionLive)
  const stoppable = useContext(StoppableTasks)
  const now = useNow(1000)
  const still = running(tasks)
  if (!live || still.length === 0) return null
  return (
    <section
      aria-label="still running beside the agent"
      data-testid="running-tasks"
      className="flex flex-col gap-1 rounded-md border border-line bg-elevated px-3 py-1.5 text-xs"
    >
      <span className="text-2xs font-medium text-fg-2">Still running beside the agent</span>
      {still.map((task) => {
        const { title, name } = labels(task)
        return (
          <div key={task.call ?? task.ids[0]} className="flex min-w-0 items-center gap-2">
            <a
              href={task.call ? `#task-${task.call}` : undefined}
              onClick={(event) => {
                const card = task.call ? document.getElementById(`task-${task.call}`) : null
                if (!card) return
                event.preventDefault()
                card.scrollIntoView({ block: 'center', behavior: 'smooth' })
              }}
              className="flex min-w-0 flex-1 items-center gap-2 text-fg-1 hover:text-accent-fg max-md:min-h-11"
            >
              <Spinner />
              <span className="shrink-0 text-fg-2">
                {task.kind === 'subagent' ? 'Subagent' : task.kind === 'background' ? 'Background' : 'Task'}
              </span>
              {name && <span className="max-w-[40%] shrink-0 truncate font-medium">{name}</span>}
              <span className="min-w-0 flex-1 truncate">{title}</span>
              <span className="shrink-0 font-mono text-fg-3 tabular-nums" data-volatile>
                {duration(task.started_ms, now) ?? '0s'}
              </span>
            </a>
            {task.call && stoppable.calls.has(task.call) && (
              <Button
                variant="ghost"
                size="sm"
                icon="stop"
                aria-label={`Stop ${name ?? title}`}
                title="End the command and what it started"
                onClick={() => stoppable.stop(task.call!)}
              >
                Stop
              </Button>
            )}
          </div>
        )
      })}
    </section>
  )
}
