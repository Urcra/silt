// Docs and sheets (ADR 0077): edited in place by people and the agent at once. Their text and cells are the thread's
// page data; each change goes to the daemon or the server as it's made, and what others change arrives with the thread.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { api, type CommentView, type SheetView } from '../api'
import { useTopic } from '../bus'
import { keys, message } from '../query'
import { columnName, splice } from '../typed'
import { Button, IconButton, Textarea, cx, toast } from '../ui'
import { CommentThread } from './CommentThread'
import { Markdown } from './Markdown'

/** Characters, as the doc counts them, in the first `units` UTF-16 units of `text`. */
function characters(text: string, units: number): number {
  return Array.from(text.slice(0, units)).length
}

/** The UTF-16 offset of character `at` of `text`. */
function units(text: string, at: number): number {
  return Array.from(text).slice(0, at).join('').length
}

/** The doc's comments, each on the text it was made on, which they follow through edits (ADR 0077). */
function DocComments({
  thread,
  comments,
  onShow,
  onChanged,
}: {
  thread: string
  comments: CommentView[]
  /** Selects a comment's text in the editor. */
  onShow: (start: number, end: number) => void
  onChanged: () => Promise<unknown>
}) {
  if (comments.length === 0) return null
  return (
    <section aria-label="comments on the doc" data-testid="doc-comments" className="mx-auto flex w-full max-w-3xl flex-col gap-2 px-6 pb-6">
      <h3 className="font-mono text-2xs tracking-[0.04em] text-fg-2 uppercase">Comments</h3>
      {comments.map((comment) => {
        const at = comment.location.at === 'doc' ? comment.location : null
        return (
          <div key={comment.id} className={cx('flex flex-col gap-1', comment.resolved && 'opacity-60')}>
            {at && (
              <button type="button" className="self-start border-l-2 border-accent pl-2 text-left text-xs text-fg-2 hover:text-fg-1" onClick={() => onShow(at.start, at.end)}>
                {at.outdated ? 'The text it was on was deleted' : `“${at.excerpt}”`}
              </button>
            )}
            <CommentThread thread={thread} comment={comment} onChanged={onChanged} />
            {!comment.resolved && (
              <Button variant="ghost" size="sm" className="self-start" onClick={() => void api.resolveComment(thread, comment.id).then(onChanged)}>
                Resolve
              </Button>
            )}
          </div>
        )
      })}
    </section>
  )
}

export function DocEditor({ thread, artifact }: { thread: string; artifact: string }) {
  useTopic(`thread:${thread}`)
  const queries = useQueryClient()
  const key = ['doc', thread, artifact]
  const doc = useQuery({ queryKey: key, queryFn: () => api.docData(thread, artifact, { op: 'read' }), refetchInterval: 5_000 })
  const [view, setView] = useState<'write' | 'read'>('read')
  const [draft, setDraft] = useState<string | null>(null)
  const comments = useQuery({ queryKey: keys.comments(thread), queryFn: () => api.comments(thread) })
  const commentsHere = (comments.data ?? []).filter((comment) => comment.location.at === 'doc' && comment.location.artifact === artifact)
  const editor = useRef<HTMLTextAreaElement>(null)
  // The characters selected in the editor, which a new comment is on.
  const [chosen, setChosen] = useState<{ start: number; end: number } | null>(null)
  const [composing, setComposing] = useState(false)
  const [commenting, setCommenting] = useState('')
  const refreshComments = () => queries.invalidateQueries({ queryKey: keys.comments(thread) })
  const choose = () => {
    const box = editor.current
    if (!box || box.selectionStart === box.selectionEnd) return setChosen(null)
    setChosen({ start: characters(box.value, box.selectionStart), end: characters(box.value, box.selectionEnd) })
  }
  const comment = async () => {
    if (!chosen || !commenting.trim()) return
    try {
      await api.addComment(thread, { on: 'doc', artifact, ...chosen }, commenting.trim())
      setCommenting('')
      setComposing(false)
      setChosen(null)
      await refreshComments()
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  const show = (start: number, end: number) => {
    setView('write')
    requestAnimationFrame(() => {
      const box = editor.current
      if (!box) return
      box.focus()
      box.setSelectionRange(units(box.value, start), units(box.value, end))
    })
  }
  // What the daemon or server last said the text is, which edits are made against.
  const synced = useRef<string | null>(null)
  const pending = useRef<number | null>(null)
  useEffect(() => {
    if (doc.data && draft === null) synced.current = doc.data.text
  }, [doc.data, draft])
  const send = async (text: string) => {
    const before = synced.current ?? ''
    const edit = splice(before, text)
    if (!edit) return
    try {
      const after = await api.docData(thread, artifact, { op: 'edit', edits: [edit] })
      synced.current = after.text
      queries.setQueryData(key, after)
      // Others' edits merged in: show the merged text unless more was typed since.
      setDraft((now) => (now === text ? null : now))
    } catch (failure) {
      toast(message(failure), 'danger')
      setDraft(null)
    }
  }
  const typed = (text: string) => {
    setDraft(text)
    if (pending.current !== null) window.clearTimeout(pending.current)
    pending.current = window.setTimeout(() => void send(text), 400)
  }
  const text = draft ?? doc.data?.text ?? ''
  if (doc.error) return <p className="px-5 py-6 text-sm text-danger">{message(doc.error)}</p>
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="doc-editor">
      <div className="flex shrink-0 items-center gap-1 border-b border-line-subtle px-2 py-1">
        <Button size="sm" variant={view === 'read' ? 'secondary' : 'ghost'} aria-pressed={view === 'read'} onClick={() => setView('read')}>
          Read
        </Button>
        <Button size="sm" variant={view === 'write' ? 'secondary' : 'ghost'} aria-pressed={view === 'write'} onClick={() => setView('write')}>
          Edit
        </Button>
        <span className="flex-1" />
        {view === 'write' && (
          <Button size="sm" variant="ghost" icon="comment" disabled={!chosen} title={chosen ? undefined : 'Select some text first'} onClick={() => setComposing(true)}>
            Comment
          </Button>
        )}
        <span className="font-mono text-2xs text-fg-3">{draft !== null ? 'Saving…' : 'Saved'}</span>
      </div>
      {view === 'write' && chosen && composing && (
        <form
          className="flex shrink-0 flex-col gap-1.5 border-b border-line-subtle px-3 py-2"
          onSubmit={(event) => {
            event.preventDefault()
            void comment()
          }}
        >
          <Textarea aria-label="comment on the selected text" rows={2} value={commenting} onChange={(event) => setCommenting(event.target.value)} placeholder="What about this text?" autoFocus />
          <div className="flex justify-end gap-1.5">
            <Button type="button" size="sm" variant="ghost" onClick={() => setComposing(false)}>
              Cancel
            </Button>
            <Button type="submit" size="sm" variant="primary" disabled={!commenting.trim()}>
              Comment
            </Button>
          </div>
        </form>
      )}
      {view === 'write' ? (
        <div className="flex min-h-0 flex-1 flex-col overflow-auto">
          <textarea
            ref={editor}
            aria-label="the doc's text"
            value={text}
            onChange={(event) => typed(event.target.value)}
            onSelect={choose}
            className="min-h-64 flex-1 resize-none bg-bg px-6 py-4 font-mono text-sm leading-6 text-fg-1 outline-none"
            spellCheck
          />
          <DocComments thread={thread} comments={commentsHere} onShow={show} onChanged={refreshComments} />
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto">
          <article className="mx-auto max-w-3xl px-6 pt-6 pb-10">
            <Markdown text={text} />
          </article>
          <DocComments thread={thread} comments={commentsHere} onShow={show} onChanged={refreshComments} />
        </div>
      )}
    </div>
  )
}

export function SheetGrid({ thread, artifact, title }: { thread: string; artifact: string; title: string }) {
  useTopic(`thread:${thread}`)
  const queries = useQueryClient()
  const key = ['sheet', thread, artifact]
  const sheet = useQuery({ queryKey: key, queryFn: () => api.sheetData(thread, artifact, { op: 'read' }) as Promise<SheetView>, refetchInterval: 5_000 })
  const [editing, setEditing] = useState<{ row: number; column: number; raw: string } | null>(null)
  const [selected, setSelected] = useState<{ row: number; column: number }>({ row: 0, column: 0 })
  const change = async (asked: Record<string, unknown>) => {
    try {
      await api.sheetData(thread, artifact, asked)
      await queries.invalidateQueries({ queryKey: key })
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  const commit = async () => {
    if (!editing) return
    const at = `${columnName(editing.column)}${editing.row + 1}`
    setEditing(null)
    await change({ op: 'set', cells: { [at]: editing.raw } })
  }
  const xlsx = async () => {
    try {
      const blob = await api.sheetXlsx(thread, artifact)
      const url = URL.createObjectURL(blob)
      const anchor = document.createElement('a')
      anchor.href = url
      anchor.download = `${title.replace(/[^A-Za-z0-9_-]+/g, '-') || 'sheet'}.xlsx`
      anchor.click()
      setTimeout(() => URL.revokeObjectURL(url), 1000)
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  if (sheet.error) return <p className="px-5 py-6 text-sm text-danger">{message(sheet.error)}</p>
  const view = sheet.data
  if (!view) return null
  const shown = view.raw[selected.row]?.[selected.column] ?? ''
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="sheet">
      <div className="flex shrink-0 items-center gap-2 border-b border-line-subtle px-2 py-1">
        <span className="w-12 font-mono text-xs text-fg-2">
          {columnName(selected.column)}
          {selected.row + 1}
        </span>
        <code className="min-w-0 flex-1 truncate font-mono text-xs text-fg-1" aria-label="the selected cell holds">
          {shown}
        </code>
        <Button size="sm" variant="ghost" onClick={() => void change({ op: 'insert', axis: 'rows', at: selected.row })}>
          Row above
        </Button>
        <Button size="sm" variant="ghost" onClick={() => void change({ op: 'insert', axis: 'columns', at: selected.column })}>
          Column left
        </Button>
        <IconButton icon="download" label="Download as .xlsx" onClick={() => void xlsx()} />
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        <table className="border-separate border-spacing-0 font-mono text-xs" role="grid" aria-label={title}>
          <thead className="sticky top-0 z-10 bg-surface-1">
            <tr>
              <th className="w-10 border-r border-b border-line" />
              {view.columns.map((id, column) => (
                <th key={id} scope="col" className="min-w-24 border-r border-b border-line px-2 py-1 font-medium text-fg-2">
                  {columnName(column)}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {view.rows.map((id, row) => (
              <tr key={id}>
                <th scope="row" className="sticky left-0 border-r border-b border-line bg-surface-1 px-2 text-right font-medium text-fg-2">
                  {row + 1}
                </th>
                {view.columns.map((columnId, column) => {
                  const active = selected.row === row && selected.column === column
                  const value = view.values[row]?.[column] ?? ''
                  return (
                    <td
                      key={columnId}
                      className={cx('h-7 border-r border-b border-line-subtle p-0', active && 'outline-2 -outline-offset-2 outline-accent')}
                      onClick={() => setSelected({ row, column })}
                      onDoubleClick={() => setEditing({ row, column, raw: view.raw[row]?.[column] ?? '' })}
                    >
                      {editing && editing.row === row && editing.column === column ? (
                        <input
                          autoFocus
                          aria-label={`cell ${columnName(column)}${row + 1}`}
                          value={editing.raw}
                          onChange={(event) => setEditing({ ...editing, raw: event.target.value })}
                          onBlur={() => void commit()}
                          onKeyDown={(event) => {
                            if (event.key === 'Enter') void commit()
                            if (event.key === 'Escape') setEditing(null)
                          }}
                          className="h-full w-full bg-bg px-2 text-fg-1 outline-none"
                        />
                      ) : (
                        <span className={cx('block truncate px-2', /^-?[\d.,]+%?$/.test(value) ? 'text-right tabular-nums' : '', value.startsWith('#') ? 'text-danger' : 'text-fg-1')}>
                          {value}
                        </span>
                      )}
                    </td>
                  )
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}
