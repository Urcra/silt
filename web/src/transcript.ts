// A thread's history as the page draws it: turns, each a prompt with what
// followed it, between dividers for compaction and for what came from
// elsewhere.

import type { HistoryEntry, TurnInfo } from './api'

export type Row =
  | { kind: 'origin'; key: string; origin: string; thread: string }
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

/** Entries the agent's tools made, folded together: its calls, their results and its thinking. */
export const FOLDED = new Set(['tool_call', 'tool_result', 'thinking', 'image'])

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
      origin = entry.origin
      current = null
      if (origin !== 'own') rows.push({ kind: 'origin', key: `origin-${entry.index}`, origin, thread: entry.thread })
    }
    if (entry.kind === 'compaction') {
      const last = rows[rows.length - 1]
      if (!isMarker(entry) && last?.kind === 'compaction' && last.summary === null) last.summary = entry
      else rows.push({ kind: 'compaction', key: `compaction-${entry.index}`, marker: isMarker(entry) ? entry : null, summary: isMarker(entry) ? null : entry })
      current = null
      continue
    }
    if (entry.kind === 'user' || current === null) {
      current = {
        kind: 'turn',
        key: `turn-${entry.index}`,
        prompt: entry.kind === 'user' ? entry : null,
        images: [],
        entries: entry.kind === 'user' ? [] : [entry],
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
  | { kind: 'tools'; entries: HistoryEntry[] }
  /** A command or a question, shown on its own rather than folded. */
  | { kind: 'step'; step: ToolStep }
  /** The turn's plan as it stood last; earlier versions of it fold away. */
  | { kind: 'plan'; entry: HistoryEntry }

/** What stands on its own in a turn rather than in a fold: commands, with their output, and questions. */
const STANDING = new Set(['execute', 'question'])

/** The pieces of a turn: its words, commands and questions, and runs of the rest of the tool work folded together. */
export function piecesOf(entries: HistoryEntry[]): Piece[] {
  const steps = stepsOf(entries)
  const standing = new Set<HistoryEntry>()
  for (const step of steps) {
    if (step.call && STANDING.has(kindOf(step.call))) {
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
      else pieces.push({ kind: 'tools', entries: [entry] })
      const into = fold ?? (pieces[pieces.length - 1] as Piece & { kind: 'tools' })
      if (entry.kind === 'tool_call') foldOf.set(entry, into)
    } else pieces.push({ kind: 'entry', entry })
  }
  return pieces
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

const SAID: Record<string, [string, string, string]> = {
  search: ['searched', 'pattern', 'patterns'],
  read: ['read', 'file', 'files'],
  execute: ['ran', 'command', 'commands'],
  edit: ['edited', 'file', 'files'],
  question: ['asked', 'question', 'questions'],
  fetch: ['fetched', 'page', 'pages'],
  plan: ['updated the plan', 'time', 'times'],
  other: ['used', 'tool', 'tools'],
}

/** What a run of tool work did, as its folded summary says it: `Searched 1 pattern, read 2 files, ran 3 commands`. */
/** A tool still waiting for its result, as a fold says it: `editing 1 file`. */
const DOING: Record<string, string> = {
  search: 'searching',
  read: 'reading',
  execute: 'running',
  edit: 'editing',
  question: 'asking',
  fetch: 'fetching',
  plan: 'updating the plan',
  other: 'using',
}

/** What a run of tool work did, as its folded summary says it: `Searched 1 pattern, read 2 files, ran 3 commands`. In a turn still `live`, calls waiting for their result say what they're doing. */
export function toolSummary(entries: HistoryEntry[], live = false): string {
  const done = new Map<string, number>()
  const doing = new Map<string, number>()
  for (const step of stepsOf(entries)) {
    const call = step.call
    if (!call) continue
    const kind = call.plan ? 'plan' : kindOf(call) in SAID ? kindOf(call) : 'other'
    const counts = step.result || !live ? done : doing
    counts.set(kind, (counts.get(kind) ?? 0) + 1)
  }
  const said = (counts: Map<string, number>, verbOf: (kind: string) => string) =>
    Object.entries(SAID)
      .filter(([kind]) => counts.has(kind))
      .map(([kind, [, one, many]]) => {
        const count = counts.get(kind)!
        const verb = verbOf(kind)
        if (kind === 'plan') return count === 1 ? verb : `${verb} ${count} ${many}`
        return `${verb} ${count} ${count === 1 ? one : many}`
      })
  const parts = [...said(done, (kind) => SAID[kind]![0]), ...said(doing, (kind) => DOING[kind]!)]
  if (parts.length === 0) return entries.some((entry) => entry.kind === 'thinking') ? 'Thought it through' : 'Tool output'
  const text = parts.join(', ')
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
    const earlier = entry.kind === 'tool_call' && entry.tool ? steps.find((step) => step.call?.tool?.id === entry.tool!.id) : undefined
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
      const before = entries.slice(0, entries.indexOf(entry)).reverse().find((candidate) => candidate.kind !== 'image')
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
