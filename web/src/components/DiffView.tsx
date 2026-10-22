import { type ReactNode, useState } from 'react'
import type { CommentView } from '../api'
import { type CodeLine, type DiffFile, type Hunk, parseFiles, splitRows } from '../diffParse'
import { type Token, languageOf, useHighlighted } from '../highlight'
import { appHref, filePath } from '../permalink'
import { message } from '../query'
import { type Lines, linesOf } from '../review'
import { Button, Counts, DiffBlock, Input, cx } from '../ui'
import { CommentThread } from './CommentThread'

export type DiffLayout = 'unified' | 'split'

/** Taking comments on a diff's new lines, and showing those already made. */
export interface Commenting {
  thread: string
  comments: CommentView[]
  onComment: (at: Lines, text: string, send: boolean) => Promise<void>
  onChanged: () => Promise<unknown>
  /** Who "send" asks, as the button names them. */
  agent?: string
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
  return <>{tokens.map((token, index) => (token.className ? <span key={index} className={token.className}>{token.text}</span> : token.text))}</>
}

function LineComment({
  at,
  agent,
  onSave,
  onCancel,
}: {
  at: Lines
  agent?: string
  onSave: (text: string, send: boolean) => Promise<void>
  onCancel: () => void
}) {
  const [text, setText] = useState('')
  const [error, setError] = useState<string>()
  const save = (send: boolean) => {
    setError(undefined)
    onSave(text, send).catch((failure: unknown) => setError(message(failure)))
  }
  return (
    <form
      className="flex flex-wrap items-center gap-2 border-y border-line-subtle bg-accent-tint px-3 py-2 font-sans"
      onSubmit={(event) => {
        event.preventDefault()
        save(false)
      }}
    >
      <Input
        aria-label="code comment"
        autoFocus
        className="min-w-48 flex-1 text-xs"
        placeholder={`Comment on ${at.path}:${at.start}${at.end !== at.start ? `-${at.end}` : ''}`}
        value={text}
        onChange={(event) => setText(event.target.value)}
      />
      {agent && (
        <Button variant="primary" disabled={!text.trim()} onClick={() => save(true)}>
          Send to {agent}
        </Button>
      )}
      <Button type="submit" disabled={!text.trim()} title="Keeps it as a comment on the lines">
        Add
      </Button>
      <Button variant="ghost" onClick={onCancel}>
        Cancel
      </Button>
      {error && <span className="w-full text-xs text-danger">{error}</span>}
    </form>
  )
}

function Gutter({ line, kind = 'context', children }: { line: number | null; kind?: CodeLine['kind']; children?: ReactNode }) {
  return <span className={cx('px-1.5 text-right text-[10px] leading-5 select-none', GUTTER[kind])}>{children ?? line ?? ''}</span>
}

function HunkView({
  file,
  hunk,
  layout,
  commenting,
}: {
  file: DiffFile
  hunk: Hunk
  layout: DiffLayout
  commenting?: Commenting
}) {
  const code = hunk.lines.map((line) => line.text).join('\n')
  const highlighted = useHighlighted(code, languageOf(file.path))
  const tokens = new Map(hunk.lines.map((line, index) => [line, highlighted?.[index]]))
  const [picking, setPicking] = useState<Lines | null>(null)
  const ending = (line: CodeLine) =>
    (commenting?.comments ?? []).filter((comment) => {
      const at = linesOf(comment)
      return at !== null && !comment.resolved && at.path === file.path && at.end === line.newLine
    })
  const picked = (line: CodeLine) =>
    picking !== null && line.newLine !== null && line.newLine >= picking.start && line.newLine <= picking.end
  const numberOf = (line: CodeLine) =>
    commenting && line.newLine !== null ? (
      <button
        type="button"
        aria-label={`comment on ${file.path} line ${line.newLine}`}
        className="hover:text-accent-fg pointer-coarse:-mx-1.5 pointer-coarse:min-w-11 pointer-coarse:px-1.5"
        onClick={(event) => {
          const here = line.newLine!
          const extend = event.shiftKey && picking !== null
          setPicking(extend ? { path: file.path, start: Math.min(picking.start, here), end: Math.max(picking.end, here) } : { path: file.path, start: here, end: here })
        }}
      >
        {line.newLine}
      </button>
    ) : (
      line.newLine
    )
  const after = (line: CodeLine) => (
    <>
      {ending(line).map((comment) => (
        <div key={comment.id} className="border-y border-line-subtle bg-accent-tint px-3 py-2 font-sans">
          <CommentThread thread={commenting!.thread} comment={comment} onChanged={commenting!.onChanged} />
        </div>
      ))}
      {picking && line.newLine === picking.end && (
        <LineComment
          at={picking}
          agent={commenting?.agent}
          onCancel={() => setPicking(null)}
          onSave={async (text, send) => {
            await commenting!.onComment(picking, text, send)
            setPicking(null)
          }}
        />
      )}
    </>
  )
  const codeCell = (line: CodeLine | null) =>
    line ? (
      <span className="min-w-0 pr-3 whitespace-pre">
        <span className={cx('select-none', line.kind === 'add' ? 'text-ok' : line.kind === 'del' ? 'text-danger' : 'text-fg-3')}>
          {SIGN[line.kind]}
        </span>
        <Tokens tokens={tokens.get(line)} text={line.text} />
      </span>
    ) : (
      <span className="bg-surface-1" />
    )
  return (
    <>
      <div className="bg-info-tint/50 px-3 font-mono text-2xs leading-6 text-info">{hunk.header}</div>
      {layout === 'unified'
        ? hunk.lines.map((line, index) => (
            <div key={index}>
              <div className={cx('grid grid-cols-[36px_36px_minmax(0,1fr)]', ROW[line.kind], picked(line) && 'ring-1 ring-accent ring-inset')}>
                <Gutter line={line.oldLine} kind={line.kind} />
                <Gutter line={line.newLine} kind={line.kind}>
                  {numberOf(line)}
                </Gutter>
                {codeCell(line)}
              </div>
              {after(line)}
            </div>
          ))
        : splitRows(hunk).map((row, index) => (
            <div key={index}>
              <div className="grid grid-cols-[36px_minmax(0,1fr)_36px_minmax(0,1fr)]">
                <Gutter line={row.left?.oldLine ?? null} kind={row.left?.kind === 'del' ? 'del' : 'context'} />
                <span className={cx('flex min-w-0 border-r border-line-subtle', row.left && row.left.kind !== 'context' && ROW.del)}>
                  {codeCell(row.left && row.left.kind !== 'add' ? row.left : null)}
                </span>
                <Gutter line={row.right?.newLine ?? null} kind={row.right?.kind === 'add' ? 'add' : 'context'}>
                  {row.right && numberOf(row.right)}
                </Gutter>
                <span className={cx('flex min-w-0', row.right && row.right.kind === 'add' && ROW.add, row.right && picked(row.right) && 'ring-1 ring-accent ring-inset')}>
                  {codeCell(row.right && row.right.kind !== 'del' ? row.right : null)}
                </span>
              </div>
              {row.right && after(row.right)}
            </div>
          ))}
    </>
  )
}

/**
 * A diff, a block per file: unified or side by side, highlighted, and with
 * `commenting`, comments taken on new lines from their numbers (shift-click
 * for a range). With `anchors`, each file's block is `#file-<path>` (ADR 0031).
 */
export function DiffView({
  text,
  layout = 'unified',
  commenting,
  anchors = false,
  picked,
  only,
  actions,
  viewFiles,
}: {
  text: string
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
}) {
  const files = parseFiles(text).filter((file) => only === undefined || file.path === only)
  if (files.length === 0) return <p className="text-sm text-fg-2">No changes.</p>
  return (
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
              <Counts added={file.added} removed={file.removed} />
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
          <div className="overflow-x-auto font-mono text-xs leading-5">
            <div className="min-w-fit">
              {file.hunks.map((hunk, index) => (
                <HunkView key={index} file={file} hunk={hunk} layout={layout} commenting={commenting} />
              ))}
            </div>
          </div>
        </DiffBlock>
      ))}
    </div>
  )
}
