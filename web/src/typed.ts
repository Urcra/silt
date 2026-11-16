// The pure parts of the artifact types' viewers (ADR 0076): parsing, the
// documents slides and boards render as, layout, and tables and CSV.

import { sandboxedDocument, type FrameTheme } from './artifacts'

export type Deck = { aspect?: '16:9' | '4:3'; style?: string; slides: { title: string; html: string; notes?: string }[] }
export type Board = { id: string; name: string; caption?: string; width: number; height: number; x?: number; y?: number; html: string }
export type Canvas = { style?: string; boards: Board[] }
export type Source = { id: string; name: string; kind: string; detail: string; at: string }
export type Series = { name: string; points: [string | number, number | null][] }
export type Block =
  | { type: 'text'; text: string }
  | { type: 'stat'; label: string; value: number | string; unit?: string; change?: number; better?: 'up' | 'down'; source: string }
  | { type: 'table'; title?: string; columns: string[]; rows: unknown[][]; source: string }
  | {
      type: 'chart'
      title?: string
      chart: string
      x?: string
      y?: string
      x_scale?: 'category' | 'number' | 'time'
      y_zero?: boolean
      stacked?: boolean
      series: Series[]
      source: string
    }
export type DataPage = { title: string; summary?: string; sources: Source[]; blocks: Block[] }

/**
 * The chart colours `--chart-1` to `--chart-6` of a version's design system, from its compiled CSS, for `theme`; none
 * where it doesn't give all six as colours, so the viewer keeps its own.
 */
export function chartColors(style: string | null | undefined, theme: 'light' | 'dark'): string[] | null {
  if (!style) return null
  const block = style.match(new RegExp(`\\[data-theme="${theme}"\\][^{]*\\{([^}]*)\\}`))?.[1]
  if (!block) return null
  const tokens = new Map([...block.matchAll(/--([\w-]+):\s*([^;]+);/g)].map((token) => [token[1], token[2].trim()]))
  const resolve = (value: string, depth = 0): string => {
    const named = value.match(/^var\(--([\w-]+)/)?.[1]
    return named && depth < 4 ? resolve(tokens.get(named) ?? '', depth + 1) : value
  }
  const colors = [1, 2, 3, 4, 5, 6].map((slot) => resolve(tokens.get(`chart-${slot}`) ?? ''))
  return colors.every((color) => /^(#[0-9a-f]{3,8}|rgba?\(|hsla?\(|oklch\(|oklab\()/i.test(color)) ? colors : null
}

/** The JSON of a typed version, or null when it isn't. */
export function parsed<T>(text: string): T | null {
  try {
    return JSON.parse(text) as T
  } catch {
    return null
  }
}

// A typed version comes from whoever wrote it, a peer's agent included, and the daemon checks only its own (ADR 0076):
// each viewer takes it only once it has the shape it draws, with lists of a size it can draw, else says it can't.
type Value = Record<string, unknown>
const object = (value: unknown): value is Value => typeof value === 'object' && value !== null && !Array.isArray(value)
const text = (value: unknown, most = 1 << 20) => typeof value === 'string' && value.length <= most
const number = (value: unknown) => typeof value === 'number' && Number.isFinite(value)
const list = (value: unknown, most: number): value is unknown[] => Array.isArray(value) && value.length <= most
const optional = (value: unknown, check: (value: unknown) => boolean) => value === undefined || value === null || check(value)

/** A deck, if `json` is one. */
export function deckOf(json: string): Deck | null {
  const deck = parsed<unknown>(json)
  if (!object(deck) || !list(deck.slides, 500)) return null
  if (!optional(deck.aspect, (aspect) => aspect === '16:9' || aspect === '4:3') || !optional(deck.style, text)) return null
  const slides = deck.slides.every((slide) => object(slide) && text(slide.title, 2000) && text(slide.html) && optional(slide.notes, text))
  return slides ? (deck as Deck) : null
}

/** A canvas, if `json` is one. */
export function canvasOf(json: string): Canvas | null {
  const canvas = parsed<unknown>(json)
  if (!object(canvas) || !list(canvas.boards, 200) || !optional(canvas.style, text)) return null
  const size = (value: unknown) => number(value) && (value as number) > 0 && (value as number) <= 10_000
  const boards = canvas.boards.every(
    (board) =>
      object(board) &&
      text(board.id, 200) &&
      text(board.name, 2000) &&
      text(board.html) &&
      size(board.width) &&
      size(board.height) &&
      optional(board.x, number) &&
      optional(board.y, number) &&
      optional(board.caption, text),
  )
  return boards ? (canvas as Canvas) : null
}

/** A data page, if `json` is one. */
export function dataPageOf(json: string): DataPage | null {
  const page = parsed<unknown>(json)
  if (!object(page) || !text(page.title, 2000) || !optional(page.summary, text)) return null
  if (!list(page.sources, 200) || !list(page.blocks, 500)) return null
  const named = (value: unknown) => text(value, 2000)
  const sources = page.sources.every(
    (source) => object(source) && named(source.id) && named(source.name) && named(source.kind) && text(source.detail) && named(source.at),
  )
  const point = (value: unknown) => list(value, 2) && value.length === 2 && (value[1] === null || number(value[1]))
  const block = (value: unknown) => {
    if (!object(value)) return false
    switch (value.type) {
      case 'text':
        return text(value.text)
      case 'stat':
        return named(value.label) && (number(value.value) || named(value.value)) && optional(value.unit, (unit) => named(unit) || number(unit)) && optional(value.change, number) && named(value.source)
      case 'table':
        return (
          optional(value.title, named) &&
          list(value.columns, 200) &&
          value.columns.every((column) => named(column) || number(column)) &&
          list(value.rows, 10_000) &&
          value.rows.every((row) => list(row, 200)) &&
          named(value.source)
        )
      case 'chart':
        return (
          optional(value.title, named) &&
          named(value.chart) &&
          optional(value.x, named) &&
          optional(value.y, named) &&
          optional(value.stacked, (stacked) => typeof stacked === 'boolean') &&
          list(value.series, 50) &&
          value.series.every((series) => object(series) && named(series.name) && list(series.points, 10_000) && series.points.every(point)) &&
          named(value.source)
        )
      default:
        return false
    }
  }
  return sources && page.blocks.every(block) ? (page as DataPage) : null
}

/**
 * One slide or board as a sealed frame's document, at its own size, on the theme's ground unless it sets its own. A
 * shared style's `@import`s go first, where CSS reads them.
 */
export function partDocument(html: string, shared: string | undefined, width: number, height: number, style: string | null, theme: FrameTheme): string {
  const imports = (shared ?? '').match(/@import\s[^;]+;/g) ?? []
  const rest = imports.reduce((left, rule) => left.replace(rule, ''), shared ?? '')
  const ground = `color-scheme:${theme};background:var(--bg,Canvas);color:var(--fg,CanvasText)`
  const css = `${imports.join('')}html,body{margin:0;width:${width}px;height:${height}px;overflow:hidden}html{${ground}}${rest}`
  return sandboxedDocument(`<!doctype html><html><head><style>${css.replace(/<\//g, '<\\/')}</style></head><body>${html}</body></html>`, {
    style,
    theme,
    parent: window.location.origin,
  })
}

/** A deck as one document to print, a slide a page at the deck's own size; scripts don't run in it. */
export function deckPrint(deck: Deck, width: number, height: number, style: string | null): string {
  const slides = deck.slides.map((slide) => `<section class="silt-slide">${slide.html}</section>`).join('')
  const css = `@page{size:${width}px ${height}px;margin:0}html,body{margin:0;padding:0}.silt-slide{position:relative;width:${width}px;height:${height}px;overflow:hidden;break-after:page}${deck.style ?? ''}${style ?? ''}`
  return `<!doctype html><html><head><meta charset="utf-8"><style>${css.replace(/<\//g, '<\\/')}</style></head><body>${slides}</body></html>`
}

/** Which slide `#n` names, counting from 1, or the first. */
export function slideFromHash(hash: string, count: number): number {
  const named = /^#(\d+)$/.exec(hash)
  const at = named ? Number(named[1]) - 1 : 0
  return Math.min(Math.max(at, 0), Math.max(count - 1, 0))
}

/** Where each board sits: where it says, else in a row after the last, 80 px apart. */
export function layout(boards: Board[]): (Board & { x: number; y: number })[] {
  let next = 0
  return boards.map((board) => {
    const x = board.x ?? next
    const y = board.y ?? 0
    next = Math.max(next, x + board.width + 80)
    return { ...board, x, y }
  })
}

/** A number as a page shows it: grouped digits, at most three decimals. */
export function shown(value: unknown): string {
  return typeof value === 'number' ? value.toLocaleString(undefined, { maximumFractionDigits: 3 }) : String(value ?? '')
}

/** A table as CSV, quoting what needs it. */
export function csvOf(columns: string[], rows: unknown[][]): string {
  const cell = (value: unknown) => {
    const text = value === null || value === undefined ? '' : String(value)
    return /[",\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text
  }
  return [columns, ...rows].map((row) => row.map(cell).join(',')).join('\n')
}

/** A chart's points as a table: x, then a column per series. */
export function chartTable(series: Series[]): { columns: string[]; rows: unknown[][] } {
  const xs: (string | number)[] = []
  for (const one of series) for (const [x] of one.points) if (!xs.includes(x)) xs.push(x)
  return {
    columns: ['x', ...series.map((one) => one.name)],
    rows: xs.map((x) => [x, ...series.map((one) => one.points.find(([px]) => px === x)?.[1] ?? null)]),
  }
}

/**
 * Where each of a chart's x values sits across its plot, 0 to 1 (ADR 0076): in order, or by value for a `number`
 * or `time` scale, which every x must then be; in order again where one isn't.
 */
export function xPositions(xs: unknown[], scale: 'category' | 'number' | 'time' | undefined): number[] {
  const ordinal = xs.map((_, at) => (xs.length === 1 ? 0.5 : at / (xs.length - 1)))
  if (!scale || scale === 'category') return ordinal
  const values = xs.map((x) => (scale === 'time' ? Date.parse(String(x)) : typeof x === 'number' ? x : Number.NaN))
  if (values.some((value) => !Number.isFinite(value))) return ordinal
  const [least, most] = [Math.min(...values), Math.max(...values)]
  return values.map((value) => (most === least ? 0.5 : (value - least) / (most - least)))
}

/** A series' runs of values, by index, so a line breaks where one is missing rather than bridging it. */
export function runs(ys: unknown[]): [number, number][][] {
  const found: [number, number][][] = []
  let run: [number, number][] = []
  ys.forEach((y, at) => {
    if (typeof y === 'number') run.push([at, y])
    else if (run.length > 0) {
      found.push(run)
      run = []
    }
  })
  if (run.length > 0) found.push(run)
  return found
}

/** The values a chart's y axis spans: from zero, or, for a line or dots that may, just around its values. */
export function yDomain(values: number[], fromZero: boolean): [number, number] {
  if (values.length === 0) return [0, 1]
  const [least, most] = [Math.min(...values), Math.max(...values)]
  if (fromZero) return [Math.min(0, least), Math.max(0, most)]
  const pad = (most - least) * 0.08 || Math.abs(most) * 0.1 || 1
  return [least - pad, most + pad]
}

/** A column's letters: 0 is `A`, 26 is `AA`. */
export function columnName(column: number): string {
  let name = ''
  for (let at = column + 1; at > 0; at = Math.floor((at - 1) / 26)) name = String.fromCharCode(65 + ((at - 1) % 26)) + name
  return name
}

/** The one splice that turns `before` into `after`: where they first differ, what goes, and what comes in. Positions
 * count characters, as the doc's text does. */
/**
 * `draft`, typed on top of `sent`, moved onto `merged`, which is `sent` with others' edits merged in: the person's
 * further change keeps its place relative to theirs, and theirs stay (ADR 0077).
 */
export function rebaseDraft(sent: string, draft: string, merged: string): string {
  const mine = splice(sent, draft)
  const theirs = splice(sent, merged)
  if (!mine) return merged
  if (!theirs) return draft
  const chars = [...merged]
  const grew = [...theirs.insert].length - theirs.delete
  const pos =
    mine.pos + mine.delete <= theirs.pos
      ? mine.pos
      : mine.pos >= theirs.pos + theirs.delete
        ? mine.pos + grew
        : // Overlapping: theirs stays whole, and the person's text goes after it.
          theirs.pos + [...theirs.insert].length
  const overlaps = !(mine.pos + mine.delete <= theirs.pos || mine.pos >= theirs.pos + theirs.delete)
  chars.splice(pos, overlaps ? 0 : mine.delete, ...mine.insert)
  return chars.join('')
}

export function splice(before: string, after: string): { pos: number; delete: number; insert: string } | null {
  const a = [...before]
  const b = [...after]
  let start = 0
  while (start < a.length && start < b.length && a[start] === b[start]) start++
  let end = 0
  while (end < a.length - start && end < b.length - start && a[a.length - 1 - end] === b[b.length - 1 - end]) end++
  if (start === a.length && start === b.length) return null
  return { pos: start, delete: a.length - start - end, insert: b.slice(start, b.length - end).join('') }
}
