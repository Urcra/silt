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
  | { type: 'stat'; label: string; value: number | string; unit?: string; change?: number; source: string }
  | { type: 'table'; title?: string; columns: string[]; rows: unknown[][]; source: string }
  | { type: 'chart'; title?: string; chart: string; x?: string; y?: string; stacked?: boolean; series: Series[]; source: string }
export type DataPage = { title: string; summary?: string; sources: Source[]; blocks: Block[] }

/** The JSON of a typed version, or null when it isn't. */
export function parsed<T>(text: string): T | null {
  try {
    return JSON.parse(text) as T
  } catch {
    return null
  }
}

/** One slide or board as a sealed frame's document, at its own size. */
export function partDocument(html: string, shared: string | undefined, width: number, height: number, style: string | null, theme: FrameTheme): string {
  const css = `html,body{margin:0;width:${width}px;height:${height}px;overflow:hidden}${shared ?? ''}`
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

/** A column's letters: 0 is `A`, 26 is `AA`. */
export function columnName(column: number): string {
  let name = ''
  for (let at = column + 1; at > 0; at = Math.floor((at - 1) / 26)) name = String.fromCharCode(65 + ((at - 1) % 26)) + name
  return name
}

/** The one splice that turns `before` into `after`: where they first differ, what goes, and what comes in. Positions
 * count characters, as the doc's text does. */
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
