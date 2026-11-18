// Work an agent started that runs beside its turn, subagents and commands
// left in the background, as everything a thread's history said of it.

import { createContext } from 'react'
import type { HistoryEntry } from './api'

export interface TaskState {
  /** The tool call that started it, when the history says. */
  call: string | null
  /** The agent's ids for it. */
  ids: string[]
  kind: 'subagent' | 'background' | null
  /** `running`, `completed`, `failed` or `stopped`. */
  status: string
  /** A subagent's transcript, by the name its entries' `side` gives. */
  side: string | null
  summary: string | null
  started_ms: number
  ended_ms: number | null
}

export interface Tasks {
  /** By the call that started each. */
  byCall: Map<string, TaskState>
  /** News of tasks whose call the history doesn't name, by the agent's id. */
  loose: Map<string, TaskState>
  /** Each subagent's entries, by its transcript's name. */
  sides: Map<string, HistoryEntry[]>
}

export const NO_TASKS: Tasks = { byCall: new Map(), loose: new Map(), sides: new Map() }

/** Folds a thread's task news, and keeps each subagent's entries apart from the thread's own. */
export function tasksOf(entries: HistoryEntry[]): Tasks {
  const tasks: Tasks = { byCall: new Map(), loose: new Map(), sides: new Map() }
  const callOfId = new Map<string, string>()
  const callOfSide = new Map<string, string>()
  for (const entry of entries) {
    if (entry.side) {
      const side = tasks.sides.get(entry.side) ?? []
      side.push(entry)
      tasks.sides.set(entry.side, side)
    }
    const news = entry.task
    if (!news) continue
    const call = news.call ?? callOfId.get(news.id) ?? (news.side ? callOfSide.get(news.side) : undefined) ?? null
    const known = call ? tasks.byCall.get(call) : tasks.loose.get(news.id)
    const state: TaskState = known ?? {
      call,
      ids: [],
      kind: null,
      status: 'running',
      side: null,
      summary: null,
      started_ms: entry.time_ms,
      ended_ms: null,
    }
    if (!state.ids.includes(news.id)) state.ids.push(news.id)
    state.kind ??= news.kind === 'subagent' || news.kind === 'background' ? news.kind : null
    state.side ??= news.side
    state.summary = news.summary ?? state.summary
    // News that it runs, such as the tie to its transcript, comes in any order and never undoes its end.
    if (news.status !== 'running' || state.ended_ms === null) {
      state.status = news.status
      state.ended_ms = news.status === 'running' ? null : (state.ended_ms ?? entry.time_ms)
    }
    if (call) {
      tasks.byCall.set(call, state)
      callOfId.set(news.id, call)
      if (state.side) callOfSide.set(state.side, call)
      // News that came before its call was named moves to it.
      const early = tasks.loose.get(news.id)
      if (early && early !== state) {
        tasks.loose.delete(news.id)
        state.summary ??= early.summary
      }
    } else tasks.loose.set(news.id, state)
  }
  return tasks
}

/** The tasks still running, oldest first. */
export function running(tasks: Tasks): TaskState[] {
  return [...tasks.byCall.values(), ...tasks.loose.values()]
    .filter((task) => task.status === 'running')
    .sort((a, b) => a.started_ms - b.started_ms)
}

/** How long before a task with no news reads as having gone quiet rather than running: its session has likely ended. */
export const QUIET_MS = 60 * 60 * 1000

/** Whether a thread whose last news came at `last_ms` may still be running what it started. */
export function mayRun(last_ms: number, agentRuns: boolean, now: number): boolean {
  return agentRuns || now - last_ms < QUIET_MS
}

/** The thread's subagents and background commands, folded from its whole history. */
export const TasksContext = createContext<Tasks>(NO_TASKS)

/** Whether the thread's session may still be running its tasks: it did something lately, or its agent runs here. */
export const SessionLive = createContext<boolean>(false)

/** The background commands the thread's agent runs here, by their calls, and how to end one. */
export interface Stoppable {
  calls: ReadonlySet<string>
  stop: (call: string) => void
}

export const StoppableTasks = createContext<Stoppable>({ calls: new Set(), stop: () => undefined })
