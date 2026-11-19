import { type ReactNode, createContext, memo, useContext, useEffect, useMemo, useRef, useState } from 'react'
import type { CommentView } from '../api'
import { type CodeLine, type DiffFile, type Hunk, LARGE_DIFF, diffSize, parseFiles, splitRows } from '../diffParse'
import { type Token, languageOf, useHighlighted } from '../highlight'
import { appHref, filePath } from '../permalink'
import { message } from '../query'
import { type Draft, type Lines, type ReviewState, type Side, linesOf, quoteOf, sideOf } from '../review'
import { useTextSettings } from '../text'
import { Button, Counts, DiffBlock, Icon, Textarea, cx } from '../ui'
import { CommentThread } from './CommentThread'

export type DiffLayout = 'unified' | 'split'

/** What a hunk's header says besides its range, such as the turn that wrote it. */
export type HunkNote = (file: DiffFile, hunk: Hunk, index: number) => ReactNode

/** The hunk a review's keys are on. */
export interface Cursor {
  path: string
  hunk: number
}

/** Lines to open a comment on, asked for by key; a new `key` asks again. */
export interface Ask {
  at: Lines
  key: number
}

/** Whether `at`'s lines are in `hunk`, on their side. */
function inHunk(hunk: Hunk, at: Lines): boolean {
  const side = sideOf(at)
  return hunk.lines.some((line) => {
    const number = side === 'old' ? (line.kind === 'add' ? null : line.oldLine) : line.newLine
    return number !== null && number >= at.start && number <= at.end
  })
}

/** Taking comments on a diff's lines, and showing those already made. */
export interface Commenting {
  thread: string
  comments: CommentView[]
  onComment: (at: Lines, text: string, send: boolean) => Promise<void>
  onChanged: () => Promise<unknown>
  /** Who "send" asks, as the button names them. */
  agent?: string
  /** Lines the change deleted take comments too, numbered as the diff's base has them. */
  deleted?: boolean
  /** A review in progress, where comments wait until it's sent (ADR 0055). */
  review?: ReviewState
}

const ROW = {
  add: 'bg-ok-tint',
  del: 'bg-danger-tint',
  context: '',
}

const SIGN = { add: '+ ', del: '- ', context: '  ' }

const GUTTER = { add: 'text-ok', del: 'text-danger', context: 'text-fg-2' }

function Tokens({ tokens, text }: { tokens: Token[] | undefined; text: string }) {
  if (!tokens) return <>{text || ' '}</>
  if (tokens.length === 0) return <> </>
  return (
    <>
      {tokens.map((token, index) =>
        token.className ? (
          <span key={index} className={token.className}>
            {token.text}
          </span>
        ) : (
          token.text
        ),
      )}
    </>
  )
}

const SHEET_BUTTON = 'justify-center max-md:text-sm'

/** A row's height at code's size, as `leading-code` draws it, for what isn't drawn yet. */
const rowPx = (codeSize: number) => (codeSize * 20) / 12

/** How a diff draws code, from text and layout (ADR 0102): whether long lines wrap, and a row's height. */
const DiffText = createContext({ wrap: true, row: rowPx(12) })
/** Past this many rows, a diff draws only what's near the view: 30,000 lines drawn at once were half a million
 * elements, seconds to open and to scroll. */
const LAZY_ROWS = 1500
/** How many of a long hunk's rows are drawn, or given up, together. */
const CHUNK_ROWS = 100
/** How far beyond the view rows are drawn ahead of it, and how much further they're kept once drawn. */
const AHEAD = '1200px 0px'
const KEPT = '4000px 0px'

/**
 * What `children` draws, only once it's near the view, and given up again once far from it, unless a comment is being
 * written in it; until then, and after, a box as tall as it was, or as `height` guesses.
 */
function NearView({ height, children }: { height: number; children: () => ReactNode }) {
  const box = useRef<HTMLDivElement>(null)
  const [near, setNear] = useState(typeof IntersectionObserver === 'undefined')
  const [tall, setTall] = useState(height)
  useEffect(() => {
    const element = box.current
    if (!element || typeof IntersectionObserver === 'undefined') return undefined
    const ahead = new IntersectionObserver(([entry]) => entry?.isIntersecting && setNear(true), { rootMargin: AHEAD })
    const kept = new IntersectionObserver(
      ([entry]) => {
        if (!entry || entry.isIntersecting || element.querySelector('textarea, input')) return
        setTall(entry.boundingClientRect.height || height)
        setNear(false)
      },
      { rootMargin: KEPT },
    )
    ahead.observe(element)
    kept.observe(element)
    return () => {
      ahead.disconnect()
      kept.disconnect()
    }
  }, [height])
  return (
    <div ref={box} style={near ? undefined : { height: tall }}>
      {near && children()}
    </div>
  )
}

/** How many rows `file` draws in `layout`. */
function rowsIn(file: DiffFile, layout: DiffLayout): number {
  return file.hunks.reduce((total, hunk) => total + 1 + (layout === 'split' ? splitRows(hunk).length : hunk.lines.length), 0)
}

/** What saving a line comment does: keep it, send it to the agent now, or add it to the review. */
type Save = 'comment' | 'send' | 'review'

function LineComment({
  at,
  agent,
  review,
  onSave,
  onCancel,
}: {
  at: Lines
  agent?: string
  /** How many comments the review in progress holds, when there's one to add to. */
  review?: number
  onSave: (text: string, how: Save) => Promise<void>
  onCancel: () => void
}) {
  const [text, setText] = useState('')
  const [error, setError] = useState<string>()
  const save = (how: Save) => {
    setError(undefined)
    onSave(text, how).catch((failure: unknown) => setError(message(failure)))
  }
  const span = `${at.start}${at.end !== at.start ? `–${at.end}` : ''}`
  const reviewing = review !== undefined
  return (
    <form
      aria-label={`comment on lines ${at.start}–${at.end}`}
      className="flex flex-col gap-2 border-y border-line-subtle bg-accent-tint px-3 py-2 font-sans max-md:fixed max-md:inset-x-0 max-md:bottom-0 max-md:z-30 max-md:border-t max-md:border-line max-md:bg-elevated max-md:px-4 max-md:pt-3 max-md:pb-[calc(env(safe-area-inset-bottom)+12px)] max-md:shadow-overlay"
      onSubmit={(event) => {
        event.preventDefault()
        save(reviewing ? 'review' : 'comment')
      }}
    >
      <span className="font-mono text-2xs text-fg-2 md:hidden">
        {at.path.split('/').pop()}:{span}
        {sideOf(at) === 'old' && ' (deleted)'}
      </span>
      <Textarea
        aria-label="code comment"
        autoFocus
        rows={2}
        className="text-xs"
        placeholder={`Comment on ${sideOf(at) === 'old' ? 'deleted ' : ''}${at.path}:${at.start}${at.end !== at.start ? `-${at.end}` : ''}`}
        value={text}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
            event.preventDefault()
            if (text.trim()) save(reviewing ? 'review' : 'comment')
          }
        }}
      />
      <div className="grid grid-cols-2 gap-2 md:flex md:justify-end">
        <Button variant="ghost" onClick={onCancel} className={SHEET_BUTTON}>
          Cancel
        </Button>
        {reviewing ? (
          <>
            <Button
              disabled={!text.trim()}
              onClick={() => save('comment')}
              title="Keeps it as a comment now, outside the review"
              className={SHEET_BUTTON}
            >
              Add single comment
            </Button>
            <Button
              type="submit"
              variant="primary"
              disabled={!text.trim()}
              title="Holds it with the review's other comments until you send them to the agent together"
              className={cx(SHEET_BUTTON, 'col-span-2')}
            >
              {review > 0 ? 'Add review comment' : 'Start a review'}
            </Button>
          </>
        ) : (
          <>
            <Button
              type="submit"
              variant={agent ? 'secondary' : 'primary'}
              disabled={!text.trim()}
              title="Keeps it as a comment on the lines"
              className={SHEET_BUTTON}
            >
              Comment
            </Button>
            {agent && (
              <Button
                variant="primary"
                disabled={!text.trim()}
                onClick={() => save('send')}
                className={cx(SHEET_BUTTON, 'col-span-2')}
              >
                Send to {agent}
              </Button>
            )}
          </>
        )}
      </div>
      {error && <span className="text-xs text-danger">{error}</span>}
    </form>
  )
}

/** A comment waiting in the review, which only this browser has until the review is sent. */
function PendingComment({ draft, review }: { draft: Draft; review: ReviewState }) {
  const [editing, setEditing] = useState(false)
  const [text, setText] = useState(draft.text)
  return (
    <div role="group" aria-label="pending comment" className="border-y border-line-subtle bg-accent-tint px-3 py-2 font-sans">
      <div className="flex flex-col gap-1 rounded-sm bg-accent-tint px-2 py-1 text-xs text-fg-1">
        <span className="flex items-center gap-2">
          <span className="rounded-sm border border-accent px-1 font-mono text-2xs text-accent-fg">Pending</span>
          {!editing && <span className="min-w-0 flex-1 whitespace-pre-wrap">{draft.text}</span>}
        </span>
        {editing ? (
          <form
            className="flex flex-col gap-1.5"
            onSubmit={(event) => {
              event.preventDefault()
              review.edit(draft.id, text.trim())
              setEditing(false)
            }}
          >
            <Textarea
              aria-label="edit pending comment"
              autoFocus
              rows={2}
              className="text-xs"
              value={text}
              onChange={(event) => setText(event.target.value)}
            />
            <span className="flex justify-end gap-2">
              <Button variant="ghost" size="sm" onClick={() => setEditing(false)}>
                Cancel
              </Button>
              <Button type="submit" size="sm" disabled={!text.trim()}>
                Update
              </Button>
            </span>
          </form>
        ) : (
          <span className="flex gap-3">
            <button type="button" className="text-accent-fg" onClick={() => setEditing(true)}>
              edit
            </button>
            <button type="button" className="text-danger" onClick={() => review.remove(draft.id)}>
              delete
            </button>
          </span>
        )}
      </div>
    </div>
  )
}

function Gutter({ line, kind = 'context', children }: { line: number | null; kind?: CodeLine['kind']; children?: ReactNode }) {
  return (
    <span className={cx('px-1.5 text-right text-gutter leading-code select-none', GUTTER[kind])}>{children ?? line ?? ''}</span>
  )
}

/** A line's number on `side`, where it has one. */
function numberOn(line: CodeLine, side: Side): number | null {
  return side === 'old' ? (line.kind === 'add' ? null : line.oldLine) : line.newLine
}

const HunkView = memo(function HunkView({
  file,
  hunk,
  layout,
  commenting,
  bare = false,
  first = false,
  index = 0,
  note,
  current = false,
  ask,
  fresh,
  lazy = false,
}: {
  file: DiffFile
  hunk: Hunk
  layout: DiffLayout
  commenting?: Commenting
  /** One gutter of line numbers and no hunk headers, for a phone's width. */
  bare?: boolean
  first?: boolean
  index?: number
  /** More in the hunk's header, such as the turn that wrote it. */
  note?: ReactNode
  /** The hunk a review's keys are on. */
  current?: boolean
  /** Lines to open a comment on, asked for by key; `key` asks again for the same ones. */
  ask?: Ask
  /** New-side lines written since the person saw the file, marked at their edge. */
  fresh?: ReadonlySet<number>
  /** Only the rows near the view are drawn. */
  lazy?: boolean
}) {
  const { wrap, row: rowHeight } = useContext(DiffText)
  const code = useMemo(() => hunk.lines.map((line) => line.text).join('\n'), [hunk])
  const highlighted = useHighlighted(code, languageOf(file.path))
  const tokens = useMemo(() => new Map(hunk.lines.map((line, index) => [line, highlighted?.[index]])), [hunk, highlighted])
  const [picking, setPicking] = useState<Lines | null>(null)
  // Where a drag from a line's "+" started, while the button is held; the box opens once it's let go, so the lines stay put.
  const dragFrom = useRef<{ side: Side; line: number } | null>(null)
  const [dragging, setDragging] = useState(false)
  // A comment asked for by key opens here once, as if its lines' number was clicked.
  const asked = ask && ask.at.path === file.path && inHunk(hunk, ask.at) ? ask : undefined
  const [answered, setAnswered] = useState<number>()
  if (asked && asked.key !== answered) {
    setAnswered(asked.key)
    setPicking(asked.at)
  }
  // Only while a drag is on: each of a large diff's thousand hunks listened all the time.
  useEffect(() => {
    if (!dragging) return undefined
    const stop = () => {
      dragFrom.current = null
      setDragging(false)
    }
    window.addEventListener('mouseup', stop)
    return () => window.removeEventListener('mouseup', stop)
  }, [dragging])
  const review = commenting?.review
  // A line takes comments on the thread's side, or on the base's when the change deleted it.
  const sideFor = (line: CodeLine): Side | null =>
    !commenting ? null : line.newLine !== null ? 'new' : commenting.deleted && line.kind === 'del' ? 'old' : null
  const pick = (side: Side, here: number, extend: boolean) =>
    setPicking((before) =>
      extend && before !== null && sideOf(before) === side
        ? { path: file.path, start: Math.min(before.start, here), end: Math.max(before.end, here), side }
        : { path: file.path, start: here, end: here, side },
    )
  const dragOver = (line: CodeLine) => {
    const from = dragFrom.current
    const here = from && numberOn(line, from.side)
    if (!from || here === null || (from.side === 'old' && line.kind !== 'del')) return
    setPicking({ path: file.path, start: Math.min(from.line, here), end: Math.max(from.line, here), side: from.side })
  }
  const picked = (line: CodeLine, side: Side | null) => {
    const number = side && numberOn(line, side)
    return picking !== null && side === sideOf(picking) && number !== null && number >= picking.start && number <= picking.end
  }
  const numberOf = (line: CodeLine, side: Side) => {
    const number = numberOn(line, side)
    if (!commenting || number === null || sideFor(line) !== side) return number
    return (
      <button
        type="button"
        aria-label={`comment on ${file.path} ${side === 'old' ? 'deleted line' : 'line'} ${number}`}
        className={cx(
          'hover:text-accent-fg',
          bare ? 'h-full w-full text-right' : 'pointer-coarse:-mx-1.5 pointer-coarse:min-w-11 pointer-coarse:px-1.5',
        )}
        onClick={(event) => pick(side, number, event.shiftKey)}
      >
        {number}
      </button>
    )
  }
  // The "+" a line shows on hover, as a forge's review does; a drag from it takes the lines it crosses.
  const plus = (line: CodeLine) => {
    const side = sideFor(line)
    const number = side && numberOn(line, side)
    if (bare || !side || number === null) return null
    return (
      <button
        type="button"
        aria-hidden="true"
        tabIndex={-1}
        title="Comment on this line; drag for more"
        className="absolute top-0.5 -left-2 z-10 hidden h-4 w-4 items-center justify-center rounded-sm bg-accent text-on-accent shadow-sm group-hover:flex pointer-coarse:!hidden"
        onMouseDown={(event) => {
          if (event.button !== 0) return
          event.preventDefault()
          dragFrom.current = { side, line: number }
          setDragging(true)
          pick(side, number, event.shiftKey)
        }}
      >
        <Icon name="plus" className="h-3 w-3" />
      </button>
    )
  }
  // The file's comments and drafts by the line they end at, found once rather than for each line.
  const comments = commenting?.comments
  const drafts = review?.review.drafts
  const endingAt = useMemo(() => {
    const found = new Map<string, CommentView[]>()
    for (const comment of comments ?? []) {
      const at = linesOf(comment)
      if (at === null || comment.resolved || at.path !== file.path) continue
      const key = `${sideOf(at)}:${at.end}`
      found.set(key, [...(found.get(key) ?? []), comment])
    }
    return found
  }, [comments, file.path])
  const draftsAt = useMemo(() => {
    const found = new Map<string, Draft[]>()
    for (const draft of drafts ?? []) {
      if (draft.at.path !== file.path) continue
      const key = `${sideOf(draft.at)}:${draft.at.end}`
      found.set(key, [...(found.get(key) ?? []), draft])
    }
    return found
  }, [drafts, file.path])
  const ending = (side: Side, number: number) => endingAt.get(`${side}:${number}`) ?? []
  const after = (line: CodeLine) => {
    const side = sideFor(line)
    const number = side && numberOn(line, side)
    if (!commenting || !side || number === null) return null
    const pending = draftsAt.get(`${side}:${number}`) ?? []
    return (
      <>
        {ending(side, number).map((comment) => (
          <div key={comment.id} className="border-y border-line-subtle bg-accent-tint px-3 py-2 font-sans">
            <CommentThread thread={commenting.thread} comment={comment} onChanged={commenting.onChanged} />
          </div>
        ))}
        {review && pending.map((draft) => <PendingComment key={draft.id} draft={draft} review={review} />)}
        {picking && !dragging && sideOf(picking) === side && picking.end === number && (
          <LineComment
            at={picking}
            agent={commenting.agent}
            review={review?.review.drafts.length}
            onCancel={() => setPicking(null)}
            onSave={async (text, how) => {
              if (how === 'review' && review) review.add(picking, text.trim(), quoteOf([file], picking))
              else await commenting.onComment(picking, text, how === 'send')
              setPicking(null)
            }}
          />
        )}
      </>
    )
  }
  // Long lines wrap rather than run past the panel's edge, where they were cut off; unless text and layout say not
  // to, when a unified diff scrolls sideways as one. Side by side, each half always wraps.
  const codeCell = (line: CodeLine | null, withPlus = true, wraps = true) =>
    line ? (
      <span className={cx('relative min-w-0 pr-3', wraps ? 'whitespace-pre-wrap [overflow-wrap:anywhere]' : 'whitespace-pre')}>
        {withPlus && plus(line)}
        <span className={cx('select-none', line.kind === 'add' ? 'text-ok' : line.kind === 'del' ? 'text-danger' : 'text-fg-3')}>
          {SIGN[line.kind]}
        </span>
        <Tokens tokens={tokens.get(line)} text={line.text} />
      </span>
    ) : (
      <span className="bg-surface-1" />
    )
  const ring = 'ring-1 ring-accent ring-inset'
  const isFresh = (line: CodeLine | null) => line !== null && line.newLine !== null && fresh?.has(line.newLine) === true
  const freshEdge = 'shadow-[inset_3px_0_0_var(--info)]'
  const bareRow = (line: CodeLine, index: number) => (
    <div key={index}>
      <div
        className={cx(
          'grid pointer-coarse:leading-7',
          wrap ? 'grid-cols-[32px_minmax(0,1fr)]' : 'grid-cols-[32px_minmax(max-content,1fr)]',
          ROW[line.kind],
          picked(line, sideFor(line)) && ring,
        )}
      >
        <Gutter line={line.newLine ?? line.oldLine} kind={line.kind}>
          {sideFor(line) && numberOf(line, sideFor(line)!)}
        </Gutter>
        {codeCell(line, true, wrap)}
      </div>
      {after(line)}
    </div>
  )
  const unifiedRow = (line: CodeLine, index: number) => (
    <div key={index}>
      <div
        className={cx(
          'group grid',
          wrap ? 'grid-cols-[36px_36px_minmax(0,1fr)]' : 'grid-cols-[36px_36px_minmax(max-content,1fr)]',
          ROW[line.kind],
          picked(line, sideFor(line)) && ring,
          isFresh(line) && freshEdge,
        )}
        onMouseEnter={() => dragOver(line)}
      >
        <Gutter line={line.oldLine} kind={line.kind}>
          {line.kind === 'del' ? numberOf(line, 'old') : undefined}
        </Gutter>
        <Gutter line={line.newLine} kind={line.kind}>
          {numberOf(line, 'new')}
        </Gutter>
        {codeCell(line, true, wrap)}
      </div>
      {after(line)}
    </div>
  )
  const splitRow = (row: ReturnType<typeof splitRows>[number], index: number) => {
    const left = row.left && row.left.kind !== 'add' ? row.left : null
    const right = row.right && row.right.kind !== 'del' ? row.right : null
    return (
      <div key={index}>
        <div className="grid grid-cols-[36px_minmax(0,1fr)_36px_minmax(0,1fr)]">
          <Gutter line={row.left?.oldLine ?? null} kind={row.left?.kind === 'del' ? 'del' : 'context'}>
            {left?.kind === 'del' ? numberOf(left, 'old') : undefined}
          </Gutter>
          <span
            className={cx(
              'group flex min-w-0 border-r border-line-subtle',
              left && left.kind !== 'context' && ROW.del,
              left?.kind === 'del' && picked(left, 'old') && ring,
            )}
            onMouseEnter={() => left && dragOver(left)}
          >
            {codeCell(left, left?.kind === 'del')}
          </span>
          <Gutter line={row.right?.newLine ?? null} kind={row.right?.kind === 'add' ? 'add' : 'context'}>
            {right && numberOf(right, 'new')}
          </Gutter>
          <span
            className={cx(
              'group flex min-w-0',
              right && right.kind === 'add' && ROW.add,
              right && picked(right, 'new') && ring,
              isFresh(right) && freshEdge,
            )}
            onMouseEnter={() => right && dragOver(right)}
          >
            {codeCell(right)}
          </span>
        </div>
        {left?.kind === 'del' && after(left)}
        {right && after(right)}
      </div>
    )
  }
  const splitRowsOf = useMemo(() => (layout === 'split' && !bare ? splitRows(hunk) : []), [layout, bare, hunk])
  // A long hunk's rows near the view, a chunk at a time, in a diff too large to draw whole.
  const drawn = (count: number, row: (index: number) => ReactNode) => {
    const all = (from: number, to: number) => Array.from({ length: to - from }, (_, at) => row(from + at))
    if (!lazy || count <= CHUNK_ROWS) return all(0, count)
    return Array.from({ length: Math.ceil(count / CHUNK_ROWS) }, (_, chunk) => {
      const from = chunk * CHUNK_ROWS
      const to = Math.min(count, from + CHUNK_ROWS)
      return (
        <NearView key={from} height={(to - from) * rowHeight}>
          {() => all(from, to)}
        </NearView>
      )
    })
  }
  return (
    <>
      {!bare && (
        <div
          data-hunk={`${file.path}#${index}`}
          className={cx(
            'flex items-center gap-3 bg-info-tint/50 px-3 font-mono text-2xs leading-6 text-info',
            current && 'ring-2 ring-accent ring-inset',
          )}
        >
          <span className="min-w-0 truncate">{hunk.header}</span>
          {note && <span className="ml-auto flex min-w-0 shrink-0 items-center gap-1 font-sans">{note}</span>}
        </div>
      )}
      {bare && !first && <div aria-hidden="true" className="h-2 border-y border-line-subtle bg-surface-1" />}
      {drawn(bare || layout === 'unified' ? hunk.lines.length : splitRowsOf.length, (index) =>
        bare
          ? bareRow(hunk.lines[index]!, index)
          : layout === 'unified'
            ? unifiedRow(hunk.lines[index]!, index)
            : splitRow(splitRowsOf[index]!, index),
      )}
    </>
  )
})

/** A file's hunks, or for one too large to draw unasked, its size and a way to draw it anyway. */
function FileHunks({
  file,
  layout,
  commenting,
  bare = false,
  note,
  cursor,
  ask,
  fresh,
  lazy = false,
}: {
  file: DiffFile
  layout: DiffLayout
  commenting?: Commenting
  bare?: boolean
  note?: HunkNote
  cursor?: Cursor
  ask?: Ask
  fresh?: ReadonlySet<number>
  lazy?: boolean
}) {
  const size = diffSize(file)
  const [shown, setShown] = useState(size <= LARGE_DIFF)
  if (!shown)
    return (
      <div className="flex flex-wrap items-center gap-2 px-3 py-2 font-sans text-xs text-fg-2">
        <span>{Math.round(size / 1024).toLocaleString()} KB of changes, too large to show here unasked.</span>
        <Button variant="ghost" onClick={() => setShown(true)}>
          Show anyway
        </Button>
      </div>
    )
  return file.hunks.map((hunk, index) => (
    <HunkView
      key={index}
      file={file}
      hunk={hunk}
      layout={layout}
      commenting={commenting}
      bare={bare}
      first={index === 0}
      index={index}
      note={note?.(file, hunk, index)}
      current={cursor?.path === file.path && cursor.hunk === index}
      ask={ask}
      fresh={fresh}
      lazy={lazy}
    />
  ))
}

/**
 * A diff, a block per file: unified or side by side, highlighted, and with
 * `commenting`, comments taken on new lines from their numbers (shift-click
 * for a range). With `anchors`, each file's block is `#file-<path>` (ADR 0031).
 */
export function DiffView({
  text,
  files: given,
  layout = 'unified',
  commenting,
  anchors = false,
  picked,
  only,
  actions,
  viewFiles,
  bare = false,
  hunkNote,
  cursor,
  ask,
  fresh,
}: {
  /** The diff, or its files already parsed. */
  text?: string
  files?: DiffFile[]
  layout?: DiffLayout
  commenting?: Commenting
  anchors?: boolean
  picked?: string
  /** Just this file. */
  only?: string
  /** More in each file's header, such as a Seen box. */
  actions?: (file: DiffFile) => ReactNode
  /** The thread whose files "view file" opens. */
  viewFiles?: string
  /** Just the lines, edge to edge, under a header drawn elsewhere: a phone's one file at a time. */
  bare?: boolean
  hunkNote?: HunkNote
  cursor?: Cursor
  ask?: Ask
  /** A file's lines written since the person saw it. */
  fresh?: (path: string) => ReadonlySet<number> | undefined
}) {
  const parsed = useMemo(() => given ?? parseFiles(text ?? ''), [given, text])
  const files = useMemo(() => parsed.filter((file) => only === undefined || file.path === only), [parsed, only])
  const lazy = useMemo(() => files.reduce((total, file) => total + rowsIn(file, layout), 0) > LAZY_ROWS, [files, layout])
  const { shown } = useTextSettings()
  const drawing = useMemo(() => ({ wrap: shown.wrap, row: rowPx(shown.code_size) }), [shown.wrap, shown.code_size])
  if (files.length === 0) return <p className="text-sm text-fg-2">No changes.</p>
  if (bare)
    return (
      <DiffText.Provider value={drawing}>
        <div data-testid="diff" className="overflow-x-auto font-mono text-code">
          <div className="min-w-fit">
            {files.map((file) => (
              <FileHunks key={file.path} file={file} layout="unified" commenting={commenting} bare />
            ))}
          </div>
        </div>
      </DiffText.Provider>
    )
  return (
    <DiffText.Provider value={drawing}>
      <div data-testid="diff" className="flex flex-col gap-3">
        {files.map((file) => (
          <DiffBlock
            key={file.path}
            id={anchors ? `file-${file.path}` : undefined}
            className={cx(file.path === picked && 'ring-2 ring-accent')}
            path={
              <>
                {file.path}
                {file.note && <span className="ml-2 text-fg-2">{file.note}</span>}
              </>
            }
            actions={
              <>
                <Counts added={file.added} removed={file.removed} binary={file.binary} />
                {viewFiles && (
                  <a
                    aria-label="view file"
                    className="font-sans text-accent-fg hover:text-fg-1 max-md:inline-flex max-md:min-h-11 max-md:items-center"
                    href={appHref(filePath(viewFiles, file.path))}
                  >
                    View file
                  </a>
                )}
                {actions?.(file)}
              </>
            }
          >
            <div
              className="overflow-x-auto font-mono text-code [content-visibility:auto]"
              style={{ containIntrinsicSize: `auto ${heightOf(file, layout, drawing.row)}px` }}
            >
              <div className="min-w-fit">
                {lazy ? (
                  <NearView height={heightOf(file, layout, drawing.row)}>
                    {() => (
                      <FileHunks
                        file={file}
                        layout={layout}
                        commenting={commenting}
                        note={hunkNote}
                        cursor={cursor}
                        ask={ask}
                        fresh={fresh?.(file.path)}
                        lazy
                      />
                    )}
                  </NearView>
                ) : (
                  <FileHunks
                    file={file}
                    layout={layout}
                    commenting={commenting}
                    note={hunkNote}
                    cursor={cursor}
                    ask={ask}
                    fresh={fresh?.(file.path)}
                  />
                )}
              </div>
            </div>
          </DiffBlock>
        ))}
      </div>
    </DiffText.Provider>
  )
}

/** About how tall a file's lines are, in pixels, until they're first drawn. Files out of view aren't laid out or
 * painted (`content-visibility`): with thousands of lines, each frame of a sidebar or panel dragged took 100 ms. */
function heightOf(file: DiffFile, layout: DiffLayout, row: number): number {
  if (diffSize(file) > LARGE_DIFF) return 40
  return rowsIn(file, layout) * row
}
