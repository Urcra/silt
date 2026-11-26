import { type ReactNode, useLayoutEffect, useRef, useState } from 'react'
import type { AutoCheck, Judge } from '../api'
import { sameJudge } from '../judges'
import { Icon, cx } from '../ui'
import { AgentMark } from './AgentMark'

/** An agent that may check for Auto, with the models it offers, or why the workspace refuses it. */
export interface JudgeAgent {
  id: string
  name: string
  models: { id: string; name: string }[]
  refused?: string
}

/** The model part of a judge's name, `Claude Sonnet 5.5` of `GitHub Copilot on Claude Sonnet 5.5`. */
function modelOf(name: string): string {
  const at = name.indexOf(' on ')
  return at < 0 ? name : name.slice(at + 4)
}

const HEADING = 'px-2 pt-1.5 pb-1 text-2xs tracking-[0.06em] text-fg-3 uppercase'
const GROUP =
  'mt-1 flex items-center gap-1.5 border-t border-line-subtle px-2 pt-2 pb-1 text-2xs tracking-[0.06em] text-fg-3 uppercase'

/**
 * Under Auto in the permissions menu, who checks the commands of an agent without an auto mode of its own (ADR 0132):
 * the thread's own agent and model, or another agent its workspace allows. A desktop opens the list beside the menu; a
 * phone opens it in the menu's place.
 */
export function JudgeRow({
  check,
  agent,
  agents,
  onPick,
}: {
  check: AutoCheck
  /** The thread's agent, by its id. */
  agent: string
  agents: JudgeAgent[]
  onPick: (judge: Judge) => void
}) {
  const row = useRef<HTMLButtonElement>(null)
  const [at, setAt] = useState<{ top: number; left?: number; right?: number; width?: number; height?: number } | null>(null)
  const judgeAgent = check.judge.kind === 'thread' ? agent : check.judge.agent
  const label = check.judge.kind === 'thread' ? `Same as this thread · ${modelOf(check.name)}` : check.name
  const open = () => {
    const menu = row.current?.closest('[role="dialog"]')?.getBoundingClientRect()
    const box = row.current?.getBoundingClientRect()
    if (!menu || !box) return
    if (window.innerWidth < 768) setAt({ top: menu.top, left: menu.left, width: menu.width, height: menu.height })
    else setAt({ top: box.top - 8, right: window.innerWidth - menu.left + 4 })
  }
  if (check.fixed_by)
    return (
      <span
        title={`Set by ${check.fixed_by}`}
        className="ml-2 flex items-center gap-1.5 rounded-sm px-2 py-1 text-xs text-fg-2 max-md:min-h-11"
      >
        <span className="text-fg-3">Checked by</span>
        <AgentMark agent={judgeAgent} className="h-3 w-3" />
        <span className="min-w-0 flex-1 truncate text-fg-1">{check.name}</span>
        <Icon name="lock" className="h-3 w-3 shrink-0 text-fg-3" />
      </span>
    )
  return (
    <>
      <button
        ref={row}
        type="button"
        role="menuitem"
        aria-haspopup="menu"
        aria-expanded={at !== null}
        aria-label="who checks for Auto"
        title={check.name}
        onClick={() => (at ? setAt(null) : open())}
        className={cx(
          'ml-2 flex items-center gap-1.5 rounded-sm px-2 py-1 text-left text-xs text-fg-2 hover:bg-surface-2 focus:bg-surface-2 focus:outline-none max-md:min-h-11',
          at && 'bg-surface-2',
        )}
      >
        <span className="text-fg-3">Checked by</span>
        <AgentMark agent={judgeAgent} className="h-3 w-3" />
        <span className="min-w-0 flex-1 truncate text-fg-1">{label}</span>
        <Icon name="chevronRight" className="h-3 w-3 shrink-0 text-fg-3" />
      </button>
      {at && (
        <JudgeList
          at={at}
          check={check}
          agent={agent}
          agents={agents}
          back={() => setAt(null)}
          onPick={(judge) => {
            setAt(null)
            onPick(judge)
          }}
        />
      )}
    </>
  )
}

/** Who may check for Auto: the thread's own agent first, then each agent the workspace allows by its models. */
function JudgeList({
  at,
  check,
  agent,
  agents,
  back,
  onPick,
}: {
  at: { top: number; left?: number; right?: number; width?: number; height?: number }
  check: AutoCheck
  agent: string
  agents: JudgeAgent[]
  back: () => void
  onPick: (judge: Judge) => void
}) {
  const list = useRef<HTMLDivElement>(null)
  const phone = at.width !== undefined
  const [top, setTop] = useState(at.top)
  // Beside the menu, it stays inside the window.
  useLayoutEffect(() => {
    if (phone || !list.current) return
    const height = list.current.getBoundingClientRect().height
    setTop(Math.max(8, Math.min(at.top, window.innerHeight - 8 - height)))
  }, [at.top, phone])
  useLayoutEffect(() => {
    list.current?.querySelector<HTMLButtonElement>('[aria-checked="true"]')?.focus()
  }, [])
  const own = agents.find((each) => each.id === agent)
  const others = agents.filter((each) => each.id !== agent || each.models.length > 0)
  const item = (key: string, name: ReactNode, judge: Judge | null, says?: string, refused?: string) => {
    const checked = judge !== null && sameJudge(judge, check.judge)
    return (
      <button
        key={key}
        type="button"
        role="menuitemradio"
        aria-checked={checked}
        aria-disabled={refused ? true : undefined}
        title={refused}
        onClick={() => judge && !refused && onPick(judge)}
        className={cx(
          'flex w-full items-center gap-2.5 rounded-sm px-2 py-1.5 text-left text-sm max-md:min-h-11',
          refused ? 'cursor-not-allowed text-fg-3' : 'text-fg-1 hover:bg-surface-2 focus:bg-surface-2 focus:outline-none',
          checked && !refused && 'bg-surface-2',
        )}
      >
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate">{name}</span>
          {refused ? (
            <span className="text-xs text-warn-fg">{refused}</span>
          ) : (
            says && <span className="text-xs text-fg-3">{says}</span>
          )}
        </span>
        {checked ? (
          <Icon name="check" className="h-3.5 w-3.5 shrink-0 text-accent-fg" />
        ) : (
          <span className="h-3.5 w-3.5 shrink-0" />
        )}
      </button>
    )
  }
  const ownName = own?.name ?? 'The thread’s agent'
  return (
    <div
      ref={list}
      role="dialog"
      aria-label="who checks for Auto"
      className="fixed z-50 max-h-[calc(100dvh-60px)] max-w-[calc(100vw-16px)] overflow-y-auto rounded-md border border-line bg-elevated p-1 text-fg-1 shadow-overlay"
      style={phone ? { top: at.top, left: at.left, width: at.width, minHeight: at.height } : { top, right: at.right, width: 288 }}
    >
      <div role="menu" aria-label="who checks for Auto" className="flex flex-col">
        {phone ? (
          <button
            type="button"
            role="menuitem"
            onClick={back}
            className={cx(HEADING, 'flex items-center gap-1.5 text-left hover:text-fg-1 max-md:min-h-11')}
          >
            <Icon name="chevronRight" className="h-3 w-3 shrink-0 rotate-180" />
            Permissions · Auto’s check
          </button>
        ) : (
          <span className={HEADING}>Auto’s check</span>
        )}
        {item(
          'thread',
          'Same as this thread',
          { kind: 'thread' },
          `${ownName} on ${modelOf(check.judge.kind === 'thread' ? check.name : 'its model')}; it follows the thread’s model`,
        )}
        {others.length === 0 && (
          <p className="mt-1 border-t border-line-subtle px-2 pt-2 pb-1 text-xs text-fg-3">
            No other agent on this machine can check. Add one in New thread’s list to choose it here.
          </p>
        )}
        {others.map((each) => (
          <div key={each.id} className="flex flex-col">
            <span className={GROUP}>
              <AgentMark agent={each.id} name={each.name} className="h-3 w-3" />
              {each.name}
            </span>
            {each.refused
              ? item(each.id, each.name, null, undefined, each.refused)
              : each.models.length > 0
                ? each.models.map((model) =>
                    item(`${each.id}/${model.id}`, model.name, { kind: 'agent', agent: each.id, model: model.id }),
                  )
                : item(each.id, each.name, { kind: 'agent', agent: each.id }, 'On the model it starts with')}
          </div>
        ))}
        <span className="mt-1 border-t border-line-subtle px-2 pt-2 pb-1 text-2xs text-fg-3">
          Each check is a session of its own, on that agent’s plan
        </span>
      </div>
    </div>
  )
}
