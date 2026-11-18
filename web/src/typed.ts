// The pure parts of the artifact types' viewers (ADR 0076): parsing, the
// documents slides and boards render as, layout, and tables and CSV.

import { ARTIFACT_CSP, sandboxedDocument, withFilesInlined, type FrameTheme, type InlineFiles } from './artifacts'

export type Deck = {
  aspect?: '16:9' | '4:3'
  transition?: 'none' | 'fade'
  style?: string
  slides: { id?: string; title: string; html: string; notes?: string }[]
}
export type Board = {
  id: string
  name: string
  caption?: string
  width: number
  height: number
  x?: number
  y?: number
  grow?: boolean
  page?: string
  html: string
}
export type CanvasNote = { id?: string; text: string; x: number; y: number; w?: number; page?: string }
export type Canvas = { style?: string; boards: Board[]; notes?: CanvasNote[] }
export type Source = {
  id: string
  name: string
  kind: string
  detail: string
  at: string
  server?: string
  tool?: string
  input?: Record<string, unknown>
}
export type Series = { name: string; points: [string | number, number | null][]; pick?: string }
export type Block =
  | { type: 'text'; text: string }
  | {
      type: 'stat'
      label: string
      value: number | string
      unit?: string
      change?: number
      better?: 'up' | 'down'
      source: string
      pick?: string
    }
  | { type: 'table'; title?: string; columns: string[]; rows: unknown[][]; source: string; pick?: string }
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
  if (
    !optional(deck.aspect, (aspect) => aspect === '16:9' || aspect === '4:3') ||
    !optional(deck.transition, (transition) => transition === 'none' || transition === 'fade') ||
    !optional(deck.style, text)
  )
    return null
  const slides = deck.slides.every(
    (slide) =>
      object(slide) &&
      optional(slide.id, (id) => text(id, 64)) &&
      text(slide.title, 2000) &&
      text(slide.html) &&
      optional(slide.notes, text),
  )
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
      optional(board.grow, (grow) => typeof grow === 'boolean') &&
      optional(board.page, (page) => text(page, 64)) &&
      optional(board.caption, text),
  )
  const notes = optional(
    canvas.notes,
    (notes) =>
      list(notes, 200) &&
      notes.every(
        (note) =>
          object(note) &&
          text(note.text, 2000) &&
          number(note.x) &&
          number(note.y) &&
          optional(note.w, (w) => number(w) && (w as number) > 0 && (w as number) <= 4000) &&
          optional(note.page, (page) => text(page, 64)) &&
          optional(note.id, (id) => text(id, 64)),
      ),
  )
  return boards && notes ? (canvas as Canvas) : null
}

/** A canvas's pages by name, in the order they're first named; none when no board or note names one. */
export function pagesOf(canvas: Canvas): string[] {
  const named = [...canvas.boards, ...(canvas.notes ?? [])].map((part) => part.page)
  if (named.every((page) => page === undefined)) return []
  const pages: string[] = []
  // What names no page is on the first, which keeps its own name if something on it gives one.
  for (const page of named) if (!pages.includes(page ?? '')) pages.push(page ?? '')
  const first = named.find((page) => page !== undefined) ?? ''
  return pages[0] === ''
    ? [first, ...pages.slice(1).filter((page) => page !== first && page !== '')]
    : pages.filter((page) => page !== '')
}

/** Whether `part` is on page `page` of a canvas whose pages are `pages`: what names none is on the first. */
export function onPage(part: { page?: string }, page: string, pages: string[]): boolean {
  return pages.length === 0 || (part.page ?? pages[0]) === page
}

/** A data page, if `json` is one. */
export function dataPageOf(json: string): DataPage | null {
  const page = parsed<unknown>(json)
  if (!object(page) || !text(page.title, 2000) || !optional(page.summary, text)) return null
  if (!list(page.sources, 200) || !list(page.blocks, 500)) return null
  const named = (value: unknown) => text(value, 2000)
  const sources = page.sources.every(
    (source) =>
      object(source) &&
      named(source.id) &&
      named(source.name) &&
      named(source.kind) &&
      text(source.detail) &&
      named(source.at) &&
      optional(source.server, named) &&
      optional(source.tool, named) &&
      optional(source.input, object),
  )
  const point = (value: unknown) => list(value, 2) && value.length === 2 && (value[1] === null || number(value[1]))
  const block = (value: unknown) => {
    if (!object(value)) return false
    switch (value.type) {
      case 'text':
        return text(value.text)
      case 'stat':
        return (
          named(value.label) &&
          (number(value.value) || named(value.value)) &&
          optional(value.unit, (unit) => named(unit) || number(unit)) &&
          optional(value.change, number) &&
          optional(value.pick, named) &&
          named(value.source)
        )
      case 'table':
        return (
          optional(value.title, named) &&
          list(value.columns, 200) &&
          value.columns.every((column) => named(column) || number(column)) &&
          list(value.rows, 10_000) &&
          value.rows.every((row) => list(row, 200)) &&
          optional(value.pick, named) &&
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
          value.series.every(
            (series) =>
              object(series) &&
              named(series.name) &&
              list(series.points, 10_000) &&
              series.points.every(point) &&
              optional(series.pick, named),
          ) &&
          named(value.source)
        )
      default:
        return false
    }
  }
  return sources && page.blocks.every(block) ? (page as DataPage) : null
}

/** How many click steps a slide has: the highest `data-step` of its elements, up to 20. */
export function stepsOf(html: string): number {
  let most = 0
  for (const found of html.matchAll(/data-step\s*=\s*["']?(\d+)/g)) most = Math.max(most, Math.min(Number(found[1]), 20))
  return most
}

/**
 * One slide or board as a sealed frame's document, at its own size, on the theme's ground unless it sets its own. A
 * shared style's `@import`s go first, where CSS reads them. With `step`, a slide's `data-step` elements past it are
 * hidden, as a presenter reveals them a click at a time.
 */
export function partDocument(
  html: string,
  shared: string | undefined,
  width: number,
  height: number,
  style: string | null,
  theme: FrameTheme,
  step?: number,
  grow?: boolean,
): string {
  return sandboxedDocument(partPage(html, shared, width, height, theme, step, grow), {
    style,
    theme,
    parent: window.location.origin,
  })
}

function partPage(
  html: string,
  shared: string | undefined,
  width: number,
  height: number,
  theme: FrameTheme,
  step?: number,
  grow?: boolean,
): string {
  const imports = (shared ?? '').match(/@import\s[^;]+;/g) ?? []
  const rest = imports.reduce((left, rule) => left.replace(rule, ''), shared ?? '')
  const ground = `color-scheme:${theme};background:var(--bg,Canvas);color:var(--fg,CanvasText)`
  const shown = Array.from({ length: step ?? 0 }, (_, at) => `[data-step="${at + 1}"]`).join(',')
  const steps = step === undefined ? '' : `[data-step]{visibility:hidden}${shown ? `${shown}{visibility:visible}` : ''}`
  // A board that grows says how tall its content is, which its viewer takes from that frame alone.
  const size = grow ? `width:${width}px;min-height:${height}px` : `width:${width}px;height:${height}px`
  const css = `${imports.join('')}html,body{margin:0;${size};overflow:hidden}html{${ground}}${rest}${steps}`
  const tell = grow
    ? `<script>new ResizeObserver(function(){parent.postMessage({silt:'grown',height:document.documentElement.scrollHeight},'*')}).observe(document.documentElement)</script>`
    : ''
  return `<!doctype html><html><head><style>${css.replace(/<\//g, '<\\/')}</style></head><body>${html}${tell}</body></html>`
}

/**
 * A deck's slides in one sealed frame's document, all of them at the deck's size, each hidden until the page holding
 * it says `{silt: 'show', at, step, fade}`; the frame hands that page its keys and clicks. One frame per deck, so a
 * deck's scripts and fonts load once (ADR 0076). `files` are the version's own, carried inside it.
 */
export function deckDocument(
  deck: Deck,
  style: string | null,
  theme: FrameTheme,
  options: { offline?: boolean; files?: InlineFiles } = {},
): string {
  const [width, height] = deck.aspect === '4:3' ? [1024, 768] : [1280, 720]
  const imports = (deck.style ?? '').match(/@import\s[^;]+;/g) ?? []
  const rest = imports.reduce((left, rule) => left.replace(rule, ''), deck.style ?? '')
  const css =
    `${imports.join('')}html,body{margin:0;width:${width}px;height:${height}px;overflow:hidden}` +
    `html{color-scheme:${theme};background:var(--bg,Canvas);color:var(--fg,CanvasText)}` +
    `.silt-slide{position:absolute;inset:0;overflow:hidden;display:none}.silt-slide.silt-on{display:block}` +
    `.silt-slide [data-step]{visibility:hidden}.silt-slide [data-step].silt-shown{visibility:visible}` +
    `.silt-fade{animation:silt-fade .3s ease-out}@keyframes silt-fade{from{opacity:0}to{opacity:1}}${rest}`
  const slides = deck.slides
    .map((slide, at) => `<section class="silt-slide" data-silt-slide="${at}">${slide.html}</section>`)
    .join('')
  // Inside the frame: show what the controls say, and hand them its keys and clicks.
  const inner =
    "<script>addEventListener('message',function(e){var d=e.data;if(e.source!==parent||!d||d.silt!=='show')return;" +
    "var s=document.querySelectorAll('.silt-slide');for(var i=0;i<s.length;i++){var on=i===d.at;" +
    "if(on&&d.fade&&!s[i].classList.contains('silt-on')){s[i].classList.remove('silt-fade');void s[i].offsetWidth;s[i].classList.add('silt-fade')}" +
    "s[i].classList.toggle('silt-on',on)}var t=s[d.at]?s[d.at].querySelectorAll('[data-step]'):[];" +
    "for(var j=0;j<t.length;j++)t[j].classList.toggle('silt-shown',Number(t[j].getAttribute('data-step'))<=d.step)});" +
    "addEventListener('keydown',function(e){parent.postMessage({silt:'key',key:e.key},'*')});" +
    "addEventListener('click',function(e){if(!e.target.closest('a,button,input,select,textarea'))parent.postMessage({silt:'key',key:'click'},'*')});</script>"
  const page = `<!doctype html><html><head><meta charset="utf-8"><style>${css.replace(/<\//g, '<\\/')}</style></head><body>${slides}${inner}</body></html>`
  return sandboxedDocument(
    withFilesInlined(page, options.files ?? {}),
    options.offline ? { probe: false, style } : { style, theme, parent: window.location.origin },
  )
}

/**
 * A deck as one HTML file that opens anywhere, with the viewer's controls (ADR 0076): its slides share one sealed
 * frame, as the viewer would have them, shown one at a time and scaled to fit; arrows, space, a click or a swipe step
 * through them and their click steps, and `#n` names a slide.
 */
export function deckOffline(deck: Deck, title: string, style: string | null, files: InlineFiles = {}): string {
  const [width, height] = deck.aspect === '4:3' ? [1024, 768] : [1280, 720]
  const frame = deckDocument(deck, style, 'light', { offline: true, files })
  const steps = JSON.stringify(deck.slides.map((slide) => stepsOf(slide.html)))
  const names = JSON.stringify(deck.slides.map((slide) => slide.title)).replace(/</g, '\\u003c')
  const controls =
    `var steps=${steps},names=${names},n=steps.length,at=0,step=0,fade=${deck.transition === 'fade'},W=${width},H=${height};` +
    "var f=document.getElementById('deck'),stage=document.getElementById('stage'),where=document.getElementById('at');" +
    "function fit(){var k=Math.min(stage.clientWidth/W,stage.clientHeight/H);f.style.transform='scale('+k+')'}" +
    "function show(moved){if(!n){where.textContent='No slides';return}f.contentWindow.postMessage({silt:'show',at:at,step:step,fade:fade&&moved},'*');" +
    "where.textContent=(at+1)+' / '+n+(names[at]?' · '+names[at]:'');try{history.replaceState(null,'','#'+(at+1))}catch(x){}}" +
    'function next(){if(step<steps[at])step++;else if(at<n-1){at++;step=0}else return;show(true)}' +
    'function prev(){if(step>0)step--;else if(at>0){at--;step=steps[at]}else return;show(true)}' +
    'function go(to){at=Math.max(0,Math.min(n-1,to));step=0;show(true)}' +
    "function key(k){if(k==='ArrowRight'||k==='ArrowDown'||k===' '||k==='PageDown'||k==='click')next();" +
    "else if(k==='ArrowLeft'||k==='ArrowUp'||k==='PageUp'||k==='Backspace')prev();else if(k==='Home')go(0);else if(k==='End')go(n-1)}" +
    'var named=/^#(\\d+)$/.exec(location.hash);if(named)at=Math.max(0,Math.min(n-1,Number(named[1])-1));' +
    "f.addEventListener('load',function(){show(false)});" +
    "addEventListener('message',function(e){if(e.source===f.contentWindow&&e.data&&e.data.silt==='key')key(String(e.data.key))});" +
    "addEventListener('keydown',function(e){if(e.target.tagName!=='BUTTON'||(e.key!==' '&&e.key!=='Enter'))key(e.key)});" +
    "addEventListener('hashchange',function(){var m=/^#(\\d+)$/.exec(location.hash);if(m&&Number(m[1])-1!==at)go(Number(m[1])-1)});" +
    "document.getElementById('prev').onclick=prev;document.getElementById('next').onclick=next;" +
    "var x0=null;stage.addEventListener('touchstart',function(e){x0=e.touches[0].clientX},{passive:true});" +
    "stage.addEventListener('touchend',function(e){if(x0===null)return;var dx=e.changedTouches[0].clientX-x0;x0=null;if(Math.abs(dx)>40)(dx<0?next:prev)()});" +
    "addEventListener('resize',fit);fit();"
  const quoted = frame.replace(/&/g, '&amp;').replace(/"/g, '&quot;')
  return [
    '<!doctype html>',
    '<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">',
    // The slides' frame is written into the page, so it's held to the page's policy as well as its own.
    `<meta http-equiv="Content-Security-Policy" content="${ARTIFACT_CSP}">`,
    `<title>${escapeText(title)}</title>`,
    '<style>html,body{margin:0;height:100%;background:#16161a;color:#e8e8ea;font:14px/1.4 system-ui,sans-serif}',
    '#stage{position:absolute;inset:0 0 52px 0;display:flex;align-items:center;justify-content:center;overflow:hidden}',
    `#deck{flex:none;width:${width}px;height:${height}px;border:0;background:#fff;transform-origin:center}`,
    'nav{position:absolute;left:0;right:0;bottom:0;height:52px;display:flex;gap:16px;align-items:center;justify-content:center}',
    'button{font:inherit;color:inherit;background:#2a2a30;border:1px solid #3a3a42;border-radius:6px;width:40px;height:32px;cursor:pointer}',
    'button:focus-visible{outline:2px solid #8ab4ff;outline-offset:2px}#at{min-width:12ch;text-align:center;font-variant-numeric:tabular-nums}</style></head>',
    `<body><main id="stage"><iframe id="deck" sandbox="allow-scripts" title="${escapeText(title)}" srcdoc="${quoted}"></iframe></main>`,
    '<nav aria-label="slides"><button id="prev" aria-label="Previous">‹</button><span id="at" aria-live="polite"></span><button id="next" aria-label="Next">›</button></nav>',
    `<script>${controls.replace(/<\//g, '<\\/')}</script></body></html>`,
  ].join('')
}

/** How wide a canvas's note is when it doesn't say. */
export const NOTE_WIDTH = 240

/**
 * A canvas as one HTML file that opens anywhere: each page's boards, captions and notes where the viewer puts them,
 * scaled to fit the window's width, the pages one after another. Each board is its own sealed frame, as in the viewer,
 * and one that grows takes the height its content says.
 */
export function canvasOffline(canvas: Canvas, title: string, style: string | null, files: InlineFiles = {}): string {
  const pages = pagesOf(canvas)
  const sections = (pages.length > 0 ? pages : ['']).map((page) => {
    const boards = layout(canvas.boards.filter((board) => onPage(board, page, pages))).map((board) => {
      const frame = sandboxedDocument(
        withFilesInlined(partPage(board.html, canvas.style, board.width, board.height, 'light', undefined, board.grow), files),
        {
          probe: false,
          style,
        },
      )
      const label = `${board.width}×${board.height}${board.caption ? ` · ${board.caption}` : ''}`
      return (
        `<figure style="left:${board.x}px;top:${board.y}px;width:${board.width}px">` +
        `<figcaption style="font-size:min(calc(14px / var(--k, 1)), ${board.width / 10}px)"><b>${escapeText(board.name)}</b> <span>${escapeText(label)}</span></figcaption>` +
        `<iframe sandbox="allow-scripts" title="${escapeText(board.name)}" data-least="${board.height}" style="width:${board.width}px;height:${board.height}px" srcdoc="${escapeText(frame)}"></iframe></figure>`
      )
    })
    const notes = (canvas.notes ?? [])
      .filter((note) => onPage(note, page, pages))
      .map(
        (note) =>
          `<p class="note" style="left:${note.x}px;top:${note.y}px;width:${note.w ?? NOTE_WIDTH}px">${escapeText(note.text)}</p>`,
      )
    const heading = pages.length > 1 ? `<h2>${escapeText(page)}</h2>` : ''
    return `<section>${heading}<div class="stage"><div class="plane">${boards.join('')}${notes.join('')}</div></div></section>`
  })
  // Sizes come from the laid-out boards and notes, so a board that grows, or a note's text, makes room for itself.
  const script =
    "function fit(){var s=document.querySelectorAll('.plane');for(var i=0;i<s.length;i++){var p=s[i],x0=0,y0=0,x1=1,y1=1;" +
    'for(var j=0;j<p.children.length;j++){var c=p.children[j];x0=Math.min(x0,c.offsetLeft);y0=Math.min(y0,c.offsetTop);' +
    'x1=Math.max(x1,c.offsetLeft+c.offsetWidth);y1=Math.max(y1,c.offsetTop+c.offsetHeight)}' +
    "var k=Math.min(1,p.parentNode.clientWidth/(x1-x0));p.style.transform='scale('+k+') translate('+(-x0)+'px,'+(-y0)+'px)';" +
    "p.style.setProperty('--k',k);p.parentNode.style.height=Math.ceil((y1-y0)*k)+'px'}}" +
    "addEventListener('message',function(e){var d=e.data;if(!d||d.silt!=='grown'||typeof d.height!=='number')return;" +
    "var f=document.querySelectorAll('iframe');for(var i=0;i<f.length;i++)if(f[i].contentWindow===e.source){" +
    "f[i].style.height=Math.max(Number(f[i].getAttribute('data-least')),Math.min(Math.round(d.height),8192))+'px';fit()}});" +
    "addEventListener('resize',fit);fit();"
  return [
    '<!doctype html>',
    '<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">',
    // The boards' frames are written into the page, so they're held to its policy as well as their own.
    `<meta http-equiv="Content-Security-Policy" content="${ARTIFACT_CSP}">`,
    `<title>${escapeText(title)}</title>`,
    '<style>html,body{margin:0;background:#f1efeb;color:#3d3a2a;font:14px/1.4 system-ui,sans-serif}',
    'h1{font-size:18px;font-weight:600;margin:0;padding:20px 24px 0}h2{font-size:15px;font-weight:600;margin:0;padding:0 24px}',
    'section{padding:16px 24px 32px}.stage{position:relative;margin-top:40px}.plane{position:absolute;top:0;left:0;transform-origin:0 0}',
    'figure{position:absolute;margin:0}figcaption{position:absolute;bottom:100%;left:0;margin-bottom:.5em;max-width:100%;overflow:hidden;white-space:nowrap;text-overflow:ellipsis}',
    'figcaption b{font-weight:500}figcaption span{color:#7a7262}',
    'iframe{display:block;border:0;background:#fff;box-shadow:0 1px 3px rgba(0,0,0,.12),0 4px 16px rgba(0,0,0,.08)}',
    '.note{position:absolute;margin:0;padding:12px;border-radius:2px;white-space:pre-wrap;font-size:16px;background:#fff3b0;color:#3d3a2a;box-shadow:0 1px 3px rgba(0,0,0,.15)}</style></head>',
    `<body><h1>${escapeText(title)}</h1>${sections.join('')}<script>${script}</script></body></html>`,
  ].join('')
}

function escapeText(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
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
/** The most series a chart draws in colour (ADR 0076). */
export const SERIES_SHOWN = 6

/**
 * A chart's series as it draws them: past six on a chart whose series add up (bars, areas, a segmented bar), the
 * largest five by total and the rest summed into "Other", in the page's own order so each keeps its colour; other
 * charts keep them all, a grey past the sixth. The table keeps every series.
 */
export function drawnSeries(chart: string, series: Series[]): Series[] {
  if (series.length <= SERIES_SHOWN || !['bar', 'area', 'segmented'].includes(chart)) return series
  const total = (one: Series) => one.points.reduce((sum, [, y]) => sum + (typeof y === 'number' ? y : 0), 0)
  const largest = series
    .map((one, at) => ({ at, total: total(one) }))
    .sort((a, b) => b.total - a.total || a.at - b.at)
    .slice(0, SERIES_SHOWN - 1)
  const kept = new Set(largest.map(({ at }) => at))
  const sums = new Map<string, { x: Series['points'][number][0]; y: number | null }>()
  series.forEach((one, at) => {
    if (kept.has(at)) return
    for (const [x, y] of one.points) {
      const key = JSON.stringify(x)
      const was = sums.get(key) ?? { x, y: null }
      sums.set(key, { x, y: typeof y === 'number' ? (was.y ?? 0) + y : was.y })
    }
  })
  const other: Series = { name: `Other (${series.length - kept.size})`, points: [...sums.values()].map(({ x, y }) => [x, y]) }
  return [...series.filter((_, at) => kept.has(at)), other]
}

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

/** What a JSON pointer, RFC 6901's `/a/0/b`, points at in `value`; none where nothing is there. */
export function pointer(value: unknown, path: string): unknown {
  if (path === '') return value
  if (!path.startsWith('/')) return undefined
  let at: unknown = value
  for (const raw of path.slice(1).split('/')) {
    const key = raw.replace(/~1/g, '/').replace(/~0/g, '~')
    if (Array.isArray(at)) at = at[Number(key)]
    else if (at && typeof at === 'object') at = (at as Record<string, unknown>)[key]
    else return undefined
  }
  return at
}

/** An MCP tool's result as one value: its structured content, else its first text as JSON, else that text. */
export function toolValue(result: unknown): unknown {
  const said = result as { structuredContent?: unknown; content?: { type?: string; text?: string }[] } | null
  if (said?.structuredContent !== undefined && said.structuredContent !== null) return said.structuredContent
  const text = said?.content?.find((part) => part.type === 'text')?.text
  if (text === undefined) return result
  try {
    return JSON.parse(text)
  } catch {
    return text
  }
}

/**
 * A data page with what its `mcp` sources' tools answered, by source id, put where its blocks' `pick`s point (ADR
 * 0084): a stat's value, a table's rows, a series' points. A pick that finds nothing of the right shape keeps what the
 * agent wrote.
 */
export function withLive(page: DataPage, answers: Record<string, unknown>): DataPage {
  const from = (source: string, pick: string | undefined) =>
    pick !== undefined && source in answers ? pointer(answers[source], pick) : undefined
  const blocks = page.blocks.map((block): Block => {
    if (block.type === 'stat') {
      const value = from(block.source, block.pick)
      return typeof value === 'number' || typeof value === 'string' ? { ...block, value } : block
    }
    if (block.type === 'table') {
      const rows = from(block.source, block.pick)
      return Array.isArray(rows) && rows.every((row) => Array.isArray(row) && row.length === block.columns.length)
        ? { ...block, rows: rows as unknown[][] }
        : block
    }
    if (block.type === 'chart') {
      const series = block.series.map((one) => {
        const points = from(block.source, one.pick)
        const shaped =
          Array.isArray(points) &&
          points.every(
            (point) => Array.isArray(point) && point.length === 2 && (typeof point[1] === 'number' || point[1] === null),
          )
        return shaped ? { ...one, points: points as Series['points'] } : one
      })
      return { ...block, series }
    }
    return block
  })
  return { ...page, blocks }
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
