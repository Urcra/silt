// A thread's history as the page draws it: turns, each a prompt with what
// followed it, between dividers for compaction and for what came from
// elsewhere.

import type { HistoryEntry, TurnInfo } from './api'
import { NO_TASKS, type Tasks } from './tasks'

export type Row =
  /** Where the history moves to another thread's part, or back to this fork's own; `folded` turns of it hidden behind it. */
  | { kind: 'origin'; key: string; origin: string; thread: string; folded?: number }
  | { kind: 'compaction'; key: string; marker: HistoryEntry | null; summary: HistoryEntry | null }
  | {
      kind: 'turn'
      key: string
      prompt: HistoryEntry | null
      /** The pictures the prompt carried. */
      images: HistoryEntry[]
      entries: HistoryEntry[]
      turn: TurnInfo | null
      /** What the agent's messages in the turn wrote, in tokens. */
      tokens: number
    }

/** Which part of the history a row is from: this thread's own, inherited before a fork, or pulled upstream. */
export function originOf(row: Row): string | undefined {
  if (row.kind === 'origin') return row.origin
  if (row.kind === 'compaction') return (row.marker ?? row.summary)?.origin
  return (row.prompt ?? row.entries[0])?.origin
}

/** A fork's history with what it inherited folded into the divider that says where it came from: the fork's own turns are what it is about. */
export function foldInherited(rows: Row[]): Row[] {
  const shown: Row[] = []
  let into: Extract<Row, { kind: 'origin' }> | null = null
  for (const row of rows) {
    if (row.kind === 'origin' && row.origin === 'inherited') {
      into = { ...row, folded: 0 }
      shown.push(into)
    } else if (into && originOf(row) === 'inherited') {
      if (row.kind === 'turn') into.folded = (into.folded ?? 0) + 1
    } else {
      into = null
      shown.push(row)
    }
  }
  // A fork with nothing of its own yet still says where its part will begin.
  const last = shown.at(-1)
  if (last?.kind === 'origin' && last.origin === 'inherited')
    shown.push({ kind: 'origin', key: 'origin-own-to-come', origin: 'own', thread: last.thread })
  return shown
}

/** Entries the agent's tools made, folded together: its calls, their results and its thinking. */
export const FOLDED = new Set(['tool_call', 'tool_result', 'thinking', 'image'])

/** A prompt, or the agent going on by itself without one, as Claude does once a background task ends: either starts a turn. */
function opensTurn(entry: HistoryEntry): boolean {
  return entry.kind === 'user' || entry.kind === 'wake'
}

function isMarker(entry: HistoryEntry): boolean {
  return entry.kind === 'compaction' && entry.text.startsWith('(context compacted')
}

export function rowsOf(entries: HistoryEntry[], turns: TurnInfo[]): Row[] {
  const turnAt = new Map(turns.filter((turn) => turn.entry !== null).map((turn) => [turn.entry, turn]))
  const rows: Row[] = []
  let current: Extract<Row, { kind: 'turn' }> | null = null
  let origin = 'own'
  // An agent may record one message over several lines, each with its cost.
  const counted = new Set<string>()
  for (const entry of entries) {
    // A subagent's entries show inside the call that started it, not as the thread's own turns.
    if (entry.side) continue
    if (entry.kind === 'usage') {
      if (current && entry.usage && !counted.has(entry.usage.message)) {
        counted.add(entry.usage.message)
        current.tokens += entry.usage.output
      }
      continue
    }
    if (entry.kind === 'image' && current?.prompt && current.entries.length === 0) {
      current.images.push(entry)
      continue
    }
    if (entry.origin !== origin) {
      // Where a fork's own part starts after what it inherited is marked too.
      if (entry.origin !== 'own' || origin === 'inherited')
        rows.push({ kind: 'origin', key: `origin-${entry.index}`, origin: entry.origin, thread: entry.thread })
      origin = entry.origin
      current = null
    }
    if (entry.kind === 'compaction') {
      const last = rows[rows.length - 1]
      if (!isMarker(entry) && last?.kind === 'compaction' && last.summary === null) last.summary = entry
      else
        rows.push({
          kind: 'compaction',
          key: `compaction-${entry.index}`,
          marker: isMarker(entry) ? entry : null,
          summary: isMarker(entry) ? null : entry,
        })
      current = null
      continue
    }
    if (opensTurn(entry) || current === null) {
      current = {
        kind: 'turn',
        key: `turn-${entry.index}`,
        prompt: opensTurn(entry) ? entry : null,
        images: [],
        entries: opensTurn(entry) ? [] : [entry],
        turn: turnAt.get(entry.index) ?? null,
        tokens: 0,
      }
      rows.push(current)
      continue
    }
    current.entries.push(entry)
  }
  return rows
}

export type Piece =
  | { kind: 'entry'; entry: HistoryEntry }
  /** A run of the agent's work between its words: tool calls, commands and thoughts. `until` is when what follows it began. */
  | { kind: 'tools'; entries: HistoryEntry[]; until: number | null }
  /** A question, a proposed plan or a failed command, shown on its own rather than folded. */
  | { kind: 'step'; step: ToolStep }
  /** The turn's plan as it stood last; earlier versions of it fold away. */
  | { kind: 'plan'; entry: HistoryEntry }

/** What stands on its own in a turn rather than in a run: questions, commands that failed, and work left running beside the turn. */
function stands(step: ToolStep, tasks: Tasks): boolean {
  if (!step.call) return false
  const kind = kindOf(step.call)
  return (
    kind === 'question' ||
    Boolean(proposedPlan(step.call)) ||
    (kind === 'execute' && Boolean(step.result?.outcome?.failed)) ||
    tasked(step, tasks)
  )
}

/** Whether a step started a subagent or left a command running in the background. */
export function tasked(step: ToolStep, tasks: Tasks): boolean {
  const call = step.call
  if (!call) return false
  return kindOf(call) === 'subagent' || (call.tool !== null && call.tool !== undefined && tasks.byCall.has(call.tool.id))
}

/** The plan a call put to the person for approval, as Claude's ExitPlanMode does: its input's `plan`, as Markdown. */
export function proposedPlan(call: HistoryEntry): string | null {
  const at = call.text.indexOf(' {')
  if (at < 0) return null
  try {
    const input = JSON.parse(call.text.slice(at + 1)) as { plan?: unknown }
    return typeof input.plan === 'string' ? input.plan : null
  } catch {
    return null
  }
}

export function piecesOf(entries: HistoryEntry[], tasks: Tasks = NO_TASKS): Piece[] {
  const steps = stepsOf(entries)
  const standing = new Set<HistoryEntry>()
  for (const step of steps) {
    // A proposed plan stands in its turn, as what the work after it went by; a failed command, so it isn't missed.
    if (step.call && stands(step, tasks)) {
      standing.add(step.call)
      if (step.result) standing.add(step.result)
      for (const image of step.images) standing.add(image)
    }
  }
  const plans = entries.filter((entry) => entry.plan)
  const plan = plans[plans.length - 1]
  // The turn's plan shows once, as it stood last; its earlier versions and their results leave the folds.
  const hidden = new Set<HistoryEntry>()
  for (const entry of plans) {
    if (entry === plan) continue
    hidden.add(entry)
  }
  for (const step of steps) if (step.call?.plan && step.result) hidden.add(step.result)
  // News of a subagent or background command shows on the call that started it.
  for (const entry of entries) if (entry.kind === 'task') hidden.add(entry)
  // A call an update of it replaced shows as the update.
  const calls = new Set(steps.map((step) => step.call))
  for (const entry of entries) if (entry.kind === 'tool_call' && !calls.has(entry)) hidden.add(entry)
  const pieces: Piece[] = []
  // The fold each folded call went into, so its result joins it there.
  const foldOf = new Map<HistoryEntry, Piece & { kind: 'tools' }>()
  for (const entry of entries) {
    const last = pieces[pieces.length - 1]
    if (entry === plan) pieces.push({ kind: 'plan', entry })
    else if (hidden.has(entry)) continue
    else if (standing.has(entry)) {
      const step = steps.find((candidate) => candidate.call === entry)
      if (step) pieces.push({ kind: 'step', step })
    } else if (FOLDED.has(entry.kind)) {
      const call = steps.find((step) => step.result === entry)?.call
      const fold = (call && foldOf.get(call)) ?? (last?.kind === 'tools' ? last : null)
      if (fold) fold.entries.push(entry)
      else pieces.push({ kind: 'tools', entries: [entry], until: null })
      const into = fold ?? (pieces[pieces.length - 1] as Piece & { kind: 'tools' })
      if (entry.kind === 'tool_call') foldOf.set(entry, into)
    } else pieces.push({ kind: 'entry', entry })
  }
  // A run's last thought ends where what follows the run begins.
  for (const piece of pieces) {
    if (piece.kind !== 'tools') continue
    const shown = new Set(piece.entries)
    const last = piece.entries[piece.entries.length - 1].index
    piece.until = entries.find((entry) => entry.index > last && !shown.has(entry) && !hidden.has(entry))?.time_ms ?? null
  }
  return pieces
}

/** A run's thoughts and steps in the order the agent took them; a thought is the thinking between two other things. */
export type RunItem = { kind: 'thought'; entries: HistoryEntry[]; took: string | null } | { kind: 'step'; step: ToolStep }

export function runItems(entries: HistoryEntry[], until: number | null): RunItem[] {
  const steps = stepsOf(entries)
  const anchored = new Map(steps.map((step) => [step.call ?? step.result!, step]))
  const items: RunItem[] = []
  for (const entry of entries) {
    if (entry.kind === 'thinking') {
      const last = items[items.length - 1]
      if (last?.kind === 'thought') last.entries.push(entry)
      else items.push({ kind: 'thought', entries: [entry], took: null })
      continue
    }
    const step = anchored.get(entry)
    if (step) items.push({ kind: 'step', step })
  }
  // A thought took from its first words to whatever came after it.
  for (const item of items) {
    if (item.kind !== 'thought') continue
    const first = item.entries[0]
    const lastIndex = item.entries[item.entries.length - 1].index
    const next = entries.find((entry) => entry.index > lastIndex && entry.kind !== 'thinking')
    const end = next?.time_ms ?? until
    item.took = end === null ? null : duration(first.time_ms, end)
  }
  return items
}

/** How long a run's thoughts took together, as `12s`, or null when none was timed. */
export function thoughtTotal(items: RunItem[]): string | null {
  const seconds = items.reduce((sum, item) => sum + (item.kind === 'thought' && item.took ? secondsOf(item.took) : 0), 0)
  return seconds > 0 ? duration(0, seconds * 1000) : null
}

function secondsOf(said: string): number {
  const match = /^(?:(\d+)m )?(\d+)s$/.exec(said)
  return match ? Number(match[1] ?? 0) * 60 + Number(match[2]) : 0
}

/** A thought's headline: its own bold heading, or its first sentence, as a few words to show in a fold. */
export function headline(text: string): string {
  const bold = /^\s*\*\*(.+?)\*\*/.exec(text)?.[1]
  const said = (bold ?? text).trim().replace(/\s+/g, ' ')
  const sentence = /^(.+?[.!?])(\s|$)/.exec(said)?.[1] ?? said
  return sentence.length > 160 ? `${sentence.slice(0, 157).trimEnd()}…` : sentence
}

/** A tool call's name and the file it names, from its entry's `Name {json}` text. */
export function toolOf(text: string): { name: string; file: string | null } {
  const space = text.indexOf(' ')
  const name = space < 0 ? text : text.slice(0, space)
  let file: string | null = null
  try {
    const input = JSON.parse(text.slice(space + 1)) as Record<string, unknown>
    const path = input.file_path ?? input.path ?? input.notebook_path
    if (typeof path === 'string') file = path
  } catch {
    // Not every tool takes JSON; then it names no file.
  }
  return { name, file }
}

const EDITS = new Set(['Edit', 'MultiEdit', 'Write', 'NotebookEdit', 'edit', 'write'])

/** What a tool call does, from what the history read of it, else from its name. */
export function kindOf(call: HistoryEntry): string {
  if (call.tool) return call.tool.kind
  const { name, file } = toolOf(call.text)
  return EDITS.has(name) ? 'edit' : file ? 'read' : 'other'
}

// What changed things comes first: edits, then commands, then what only looked.
const SAID: Record<string, [string, string, string]> = {
  edit: ['edited', 'file', 'files'],
  execute: ['ran', 'command', 'commands'],
  subagent: ['started', 'subagent', 'subagents'],
  question: ['asked', 'question', 'questions'],
  search: ['searched', 'pattern', 'patterns'],
  read: ['read', 'file', 'files'],
  fetch: ['fetched', 'page', 'pages'],
  plan: ['updated the plan', 'time', 'times'],
  other: ['used', 'tool', 'tools'],
}

/** A tool still waiting for its result, as a fold says it: `editing 1 file`. */
const DOING: Record<string, string> = {
  search: 'searching',
  read: 'reading',
  execute: 'running',
  subagent: 'waiting on',
  edit: 'editing',
  question: 'asking',
  fetch: 'fetching',
  plan: 'updating the plan',
  other: 'using',
}

/** What a run of tool work did, as its folded summary says it: `Searched 1 pattern, read 2 files, ran 3 commands`. In a turn still `live`, calls waiting for their result say what they're doing. */
export function toolSummary(entries: HistoryEntry[], live = false, most = Infinity): string {
  // Reading or editing one file three times is still one file.
  const done = new Map<string, Set<string>>()
  const doing = new Map<string, Set<string>>()
  for (const step of stepsOf(entries)) {
    const call = step.call
    if (!call) continue
    const kind = call.plan ? 'plan' : kindOf(call) in SAID ? kindOf(call) : 'other'
    const counts = step.result || !live ? done : doing
    const which = (kind === 'read' || kind === 'edit') && call.tool?.subject ? call.tool.subject : String(call.index)
    counts.set(kind, (counts.get(kind) ?? new Set()).add(which))
  }
  const said = (counts: Map<string, Set<string>>, verbOf: (kind: string) => string) =>
    Object.entries(SAID)
      .filter(([kind]) => counts.has(kind))
      .map(([kind, [, one, many]]) => {
        const count = counts.get(kind)!.size
        const verb = verbOf(kind)
        if (kind === 'plan') return count === 1 ? verb : `${verb} ${count} ${many}`
        return `${verb} ${count} ${count === 1 ? one : many}`
      })
  // Past `most` kinds, the rest are only counted, so a busy turn still says it in a line.
  const counted = (parts: string[], counts: Map<string, Set<string>>) => {
    if (parts.length <= most) return parts.join(', ')
    const rest = Object.keys(SAID)
      .filter((kind) => counts.has(kind))
      .slice(most)
      .reduce((sum, kind) => sum + counts.get(kind)!.size, 0)
    return [...parts.slice(0, most), `${rest} other ${rest === 1 ? 'step' : 'steps'}`].join(', ')
  }
  const finished = counted(
    said(done, (kind) => SAID[kind]![0]),
    done,
  )
  const underWay = said(doing, (kind) => DOING[kind]!).join(', ')
  const text = finished && underWay ? `${finished} · now ${underWay}` : finished || underWay
  if (!text) return entries.some((entry) => entry.kind === 'thinking') ? 'Thought' : 'Tool output'
  return text.charAt(0).toUpperCase() + text.slice(1)
}

/** A tool call and the result that answers it, or a result whose call isn't here. */
export interface ToolStep {
  call: HistoryEntry | null
  result: HistoryEntry | null
  /** The pictures its result carried. */
  images: HistoryEntry[]
}

/** A run's calls each with its result: by the call's id where the history names it, else the next result after the call. */
export function stepsOf(entries: HistoryEntry[]): ToolStep[] {
  const results = entries.filter((entry) => entry.kind === 'tool_result')
  const claimed = new Set<HistoryEntry>()
  const steps: ToolStep[] = []
  for (const entry of entries) {
    // An agent that said more of a call later, its command for one, updates the call already shown.
    const earlier =
      entry.kind === 'tool_call' && entry.tool ? steps.find((step) => step.call?.tool?.id === entry.tool!.id) : undefined
    if (earlier) {
      earlier.call = entry
      continue
    }
    if (entry.kind === 'tool_call') {
      const id = entry.tool?.id
      const result =
        results.find((candidate) => !claimed.has(candidate) && id !== undefined && candidate.outcome?.call === id) ??
        results.find((candidate) => !claimed.has(candidate) && candidate.index > entry.index && !candidate.outcome?.call)
      if (result) claimed.add(result)
      steps.push({ call: entry, result: result ?? null, images: [] })
    } else if (entry.kind === 'tool_result' && !claimed.has(entry)) {
      steps.push({ call: null, result: entry, images: [] })
    } else if (entry.kind === 'image') {
      // A result's pictures follow it.
      const before = entries
        .slice(0, entries.indexOf(entry))
        .reverse()
        .find((candidate) => candidate.kind !== 'image')
      steps.find((step) => step.result !== null && step.result === before)?.images.push(entry)
    }
  }
  // A result shown on its own before a later call claimed it by id.
  return steps.filter((step) => step.call !== null || (step.result !== null && !claimed.has(step.result)))
}

/** The last `count` lines of a command's output, and how many came before them. */
export function tailOf(text: string, count = 4): { lines: string[]; hidden: number } {
  const lines = text.replace(/\s+$/, '').split('\n')
  return { lines: lines.slice(-count), hidden: Math.max(0, lines.length - count) }
}

/** How long a turn took, from its prompt to its last entry, as `1m 12s`. */
export function duration(from: number, to: number): string | null {
  const seconds = Math.round((to - from) / 1000)
  if (seconds <= 0) return null
  return seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`
}

/** A count of tokens as people say it: `840`, `18.4k`, `1.2M`. */
export function tokens(count: number): string {
  if (count < 1000) return String(count)
  if (count < 1_000_000) return `${(count / 1000).toFixed(count < 10_000 ? 1 : 0).replace(/\.0$/, '')}k`
  return `${(count / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`
}

/** How many tests a command's output says passed and failed, as cargo, pytest and vitest sum them up. */
export function testsOf(output: string): { passed: number; failed: number } | null {
  const summary = output
    .split('\n')
    .reverse()
    .find((line) => /\b\d+ passed\b/.test(line) && /\b(test result|tests?|passed|failed)\b/i.test(line))
  if (!summary) return null
  const count = (word: string) => Number(new RegExp(`\\b(\\d+) ${word}\\b`).exec(summary)?.[1] ?? 0)
  return { passed: count('passed'), failed: count('failed') }
}

/** How big a base64 image is, as people say it: `182 KB`. */
export function imageSize(base64: string): string {
  const bytes = Math.floor((base64.length * 3) / 4)
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

/** A long command as its card's one line says it: each URL shrinks to its last segment, `…/upload`; the whole command is its title. */
export function shortCommand(command: string, max = 64): string {
  if (command.length <= max) return command
  return command.replace(/\bhttps?:\/\/\S+/g, (url) => `…/${url.replace(/\/+$/, '').split('/').pop()}`)
}

function sameList<T>(a: readonly T[], b: readonly T[]): boolean {
  return a.length === b.length && a.every((item, index) => item === b[index])
}

function sameRow(a: Row, b: Row): boolean {
  if (a.kind === 'origin') return b.kind === 'origin' && a.origin === b.origin && a.thread === b.thread && a.folded === b.folded
  if (a.kind === 'compaction') return b.kind === 'compaction' && a.marker === b.marker && a.summary === b.summary
  return (
    b.kind === 'turn' &&
    a.prompt === b.prompt &&
    a.turn === b.turn &&
    a.tokens === b.tokens &&
    sameList(a.entries, b.entries) &&
    sameList(a.images, b.images)
  )
}

/**
 * `after`, with each row that holds what it held in `before` kept as that row, and `before` itself when nothing
 * changed: a row drawn once isn't drawn again because something was added below it.
 */
export function keepRows(before: readonly Row[] | undefined, after: Row[]): Row[] {
  if (!before) return after
  const byKey = new Map(before.map((row) => [row.key, row]))
  let same = before.length === after.length
  const kept = after.map((row, index) => {
    const old = byKey.get(row.key)
    const chosen = old && sameRow(old, row) ? old : row
    if (chosen !== before[index]) same = false
    return chosen
  })
  return same ? (before as Row[]) : kept
}

/** The entries `tasksOf` reads: task news and subagents' own. */
export function taskEntries(entries: HistoryEntry[]): HistoryEntry[] {
  return entries.filter((entry) => entry.task || entry.side)
}

export { sameList }
