// Docs and sheets (ADR 0077): edited in place by people and the agent at once. Their text and cells are the thread's
// page data; each change goes to the daemon or the server as it's made, and what others change arrives with the thread.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { api, type ArtifactView, type CommentView, type SheetView } from '../api'
import { offlineHtml, textOf } from '../artifacts'
import { useTopic } from '../bus'
import { lineDiff } from '../lineDiff'
import { keys, message } from '../query'
import { hosted } from '../served'
import { columnName, rebaseDraft, splice } from '../typed'
import { Button, Icon, IconButton, Input, Menu, Textarea, cx, toast } from '../ui'
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

/** Hands the viewer a file the doc made. */
function saveFile(bytes: BlobPart, type: string, name: string) {
  const url = URL.createObjectURL(new Blob([bytes], { type }))
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = name
  anchor.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}

/** The doc's text against one of its versions, line by line. */
function DocCompare({ thread, version, text }: { thread: string; version: ArtifactView; text: string }) {
  const kept = useQuery({ queryKey: keys.artifact(thread, version.blob), queryFn: () => api.artifact(thread, version.blob), staleTime: Infinity })
  if (kept.error) return <p className="px-6 py-4 text-sm text-danger">{message(kept.error)}</p>
  if (!kept.data) return null
  const lines = lineDiff(textOf(kept.data), text)
  if (!lines) return <p className="px-6 py-4 text-sm text-fg-2">These versions are too long to compare here.</p>
  return (
    <div className="min-h-0 flex-1 overflow-auto" data-testid="doc-compare">
      <p className="mx-auto max-w-3xl px-6 pt-4 text-xs text-fg-2">
        v{version.version} against the doc now{version.note ? ` · ${version.note}` : ''}
      </p>
      <pre className="mx-auto max-w-3xl px-6 pt-2 pb-10 font-mono text-xs leading-5 whitespace-pre-wrap">
        {lines.map((line, at) => (
          <div
            key={at}
            className={cx(line.kind === 'added' && 'bg-ok-tint text-fg-1', line.kind === 'removed' && 'bg-danger-tint text-fg-2 line-through', line.kind === 'same' && 'text-fg-2')}
          >
            <span aria-hidden="true" className="inline-block w-4 select-none">
              {line.kind === 'added' ? '+' : line.kind === 'removed' ? '−' : ' '}
            </span>
            <span className="sr-only">{line.kind === 'added' ? 'added: ' : line.kind === 'removed' ? 'removed: ' : ''}</span>
            {line.text || ' '}
          </div>
        ))}
      </pre>
    </div>
  )
}

export function DocEditor({ thread, artifact, title, versions }: { thread: string; artifact: string; title: string; versions: ArtifactView[] }) {
  useTopic(`thread:${thread}`)
  const queries = useQueryClient()
  const key = ['doc', thread, artifact]
  const doc = useQuery({ queryKey: key, queryFn: () => api.docData(thread, artifact, { op: 'read' }), refetchInterval: 5_000 })
  const [view, setView] = useState<'write' | 'read' | 'compare'>('read')
  const [comparing, setComparing] = useState<ArtifactView | null>(null)
  const [saving, setSaving] = useState<string | null>(null)
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
  // What the daemon or server last said the text is, and the version it's at, which edits are counted against: an edit
  // names it, so what others wrote since merges with it rather than shifting it (ADR 0077).
  const synced = useRef<{ text: string; base: string | null } | null>(null)
  const pending = useRef<number | null>(null)
  useEffect(() => {
    if (doc.data && draft === null) synced.current = { text: doc.data.text, base: doc.data.base ?? null }
  }, [doc.data, draft])
  // The text typed and not yet sent, and whether a send is on its way: one at a time, each counted against the last's answer.
  const latest = useRef<string | null>(null)
  const sending = useRef(false)
  const send = async () => {
    const text = latest.current
    if (text === null || sending.current) return
    const before = synced.current ?? { text: '', base: null }
    const edit = splice(before.text, text)
    if (!edit) {
      latest.current = null
      return setDraft(null)
    }
    sending.current = true
    try {
      const after = await api.docData(thread, artifact, { op: 'edit', edits: [edit], base: before.base })
      // The text with others' edits merged in, which the next edit is counted against; what was typed since moves onto it.
      synced.current = { text: after.text, base: after.base ?? null }
      queries.setQueryData(key, after)
      const now = latest.current
      latest.current = now === null || now === text ? null : rebaseDraft(text, now, after.text)
      setDraft(latest.current)
    } catch (failure) {
      toast(message(failure), 'danger')
      latest.current = null
      setDraft(null)
    } finally {
      sending.current = false
    }
    if (latest.current !== null) void send()
  }
  const typed = (text: string) => {
    latest.current = text
    setDraft(text)
    if (pending.current !== null) window.clearTimeout(pending.current)
    pending.current = window.setTimeout(() => void send(), 400)
  }
  const text = draft ?? doc.data?.text ?? ''
  const saveVersion = async () => {
    try {
      const saved = await api.saveVersion(thread, artifact, saving ?? '')
      setSaving(null)
      await queries.invalidateQueries({ queryKey: keys.history(thread) })
      toast(`Saved as v${saved.version}`, 'ok')
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  const named = title.replace(/[^A-Za-z0-9_-]+/g, '-') || 'doc'
  const page = async () => {
    const { renderToStaticMarkup } = await import('react-dom/server')
    const shown = versions.at(-1)
    const rendered = renderToStaticMarkup(<Markdown text={text} />)
    return shown ? offlineHtml(shown, { media_type: 'text/markdown', data: '' }, rendered) : rendered
  }
  const pdf = async () => {
    try {
      // The printed page's margins are the page's own, as a deck's slides have none.
      const printable = (await page()).replace('</head>', '<style>@page{margin:18mm 16mm}</style></head>')
      const printed = await api.artifactPdf(thread, artifact, printable)
      saveFile(Uint8Array.from(atob(printed.data), (char) => char.charCodeAt(0)), 'application/pdf', `${named}.pdf`)
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
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
        {versions.length > 0 && (
          <Menu
            label="Compare with a version"
            align="left"
            trigger={<span className={cx('rounded-sm px-2 py-1 text-xs', view === 'compare' ? 'bg-surface-2 text-fg-1' : 'text-fg-2 hover:text-fg-1')}>Compare</span>}
            items={[...versions].reverse().map((version) => ({
              label: `v${version.version}${version.note ? ` · ${version.note}` : ''}`,
              onSelect: () => {
                setComparing(version)
                setView('compare')
              },
            }))}
          />
        )}
        <span className="flex-1" />
        <Button size="sm" variant="ghost" onClick={() => setSaving(saving === null ? '' : null)}>
          Save version
        </Button>
        <Menu
          label="Download the doc"
          trigger={<Icon name="download" className="h-4 w-4" />}
          items={[
            { label: 'Markdown', onSelect: () => saveFile(text, 'text/markdown', `${named}.md`) },
            { label: 'HTML', onSelect: () => void page().then((html) => saveFile(html, 'text/html', `${named}.html`)) },
            ...(hosted() ? [] : [{ label: 'PDF', hint: 'printed on this computer', onSelect: () => void pdf() }]),
          ]}
        />
        {view === 'write' && (
          <Button size="sm" variant="ghost" icon="comment" disabled={!chosen} title={chosen ? undefined : 'Select some text first'} onClick={() => setComposing(true)}>
            Comment
          </Button>
        )}
        <span className="font-mono text-2xs text-fg-3">{draft !== null ? 'Saving…' : 'Saved'}</span>
      </div>
      {saving !== null && (
        <form
          className="flex shrink-0 items-center gap-1.5 border-b border-line-subtle px-3 py-2"
          onSubmit={(event) => {
            event.preventDefault()
            void saveVersion()
          }}
        >
          <Input aria-label="what this version is" value={saving} onChange={(event) => setSaving(event.target.value)} placeholder="What this version is (optional)" autoFocus className="min-w-0 flex-1" />
          <Button type="submit" size="sm" variant="primary">
            Save
          </Button>
        </form>
      )}
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
      {view === 'compare' && comparing ? (
        <DocCompare thread={thread} version={comparing} text={text} />
      ) : view === 'write' ? (
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
