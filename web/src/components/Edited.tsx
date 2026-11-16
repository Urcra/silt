// Docs and sheets (ADR 0077): edited in place by people and the agent at once. Their text and cells are the thread's
// page data; each change goes to the daemon or the server as it's made, and what others change arrives with the thread.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { api, type ArtifactView, type CommentView, type SheetView } from '../api'
import { offlineHtml, textOf } from '../artifacts'
import { useTopic } from '../bus'
import { lineDiff } from '../lineDiff'
import { keys, message } from '../query'
import { printInBrowser } from '../printing'
import { LiveReplica } from '../replica'
import { hosted } from '../served'
import { columnName, csvOf, rebaseDraft, splice } from '../typed'
import { Button, Icon, IconButton, Input, Menu, Textarea, cx, toast } from '../ui'
import { CommentThread } from './CommentThread'
import { DocMarkdown } from './DocMarkdown'
import { RichDoc } from './RichDoc'
import { findAcross, renderedExcerpt } from '../textFind'

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
  const [view, setView] = useState<'write' | 'read' | 'rich' | 'compare'>('read')
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
  // On this machine the doc is typed into the browser's replica, a delta a keystroke, with no request per pause; the
  // hosted app, and a replica that couldn't open, send edits as before.
  const live = useRef<LiveReplica | null>(null)
  useEffect(() => {
    if (hosted()) return
    let gone = false
    void LiveReplica.open(thread).then(
      (replica) => {
        if (!gone && replica.writable) live.current = replica
      },
      () => undefined,
    )
    return () => {
      gone = true
      live.current = null
    }
  }, [thread])
  // What the editor showed when this burst of typing began, and whether the replica was set to it.
  const burst = useRef<{ base: string; begun: boolean } | null>(null)
  const typing = useRef<Promise<void>>(Promise.resolve())
  const typeLive = (replica: LiveReplica) => {
    typing.current = typing.current
      .then(async () => {
        const text = latest.current
        const begun = burst.current
        if (text === null || !begun) return
        let shown = text
        if (!begun.begun) {
          // The replica takes in what arrived since, and the typing moves onto it.
          await replica.sync()
          const now = replica.pageShow(artifact)
          begun.begun = true
          if (now !== begun.base) {
            shown = rebaseDraft(begun.base, text, now)
            latest.current = shown
            setDraft(shown)
          }
        }
        await replica.pageEdit(artifact, shown)
        if (latest.current !== shown) return
        latest.current = null
        burst.current = null
        await queries.invalidateQueries({ queryKey: key })
        if (latest.current === null) setDraft(null)
      })
      .catch((failure: unknown) => {
        toast(message(failure), 'danger')
        latest.current = null
        burst.current = null
        setDraft(null)
      })
  }
  const typed = (text: string) => {
    latest.current = text
    setDraft(text)
    const replica = live.current
    if (replica) {
      burst.current ??= { base: doc.data?.text ?? '', begun: false }
      return typeLive(replica)
    }
    if (pending.current !== null) window.clearTimeout(pending.current)
    pending.current = window.setTimeout(() => void send(), 400)
  }
  const text = draft ?? doc.data?.text ?? ''
  // The text that open comments are on, highlighted where the read view shows it.
  const reading = useRef<HTMLElement>(null)
  useEffect(() => {
    const article = reading.current
    const registry = typeof CSS !== 'undefined' ? (CSS as unknown as { highlights?: Map<string, unknown> }).highlights : undefined
    if (view !== 'read' || !article || !registry || typeof Highlight === 'undefined') return
    const nodes: Text[] = []
    const walker = document.createTreeWalker(article, NodeFilter.SHOW_TEXT)
    for (let node = walker.nextNode(); node; node = walker.nextNode()) nodes.push(node as Text)
    const texts = nodes.map((node) => node.data)
    const ranges = commentsHere.flatMap((comment) => {
      if (comment.resolved || comment.location.at !== 'doc' || comment.location.outdated) return []
      const found = findAcross(texts, renderedExcerpt(comment.location.excerpt))
      if (!found) return []
      const range = document.createRange()
      range.setStart(nodes[found.start[0]], found.start[1])
      range.setEnd(nodes[found.end[0]], found.end[1])
      return [range]
    })
    registry.set('silt-doc-comments', new Highlight(...ranges))
    return () => {
      registry.delete('silt-doc-comments')
    }
  }, [view, text, commentsHere])
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
    const rendered = renderToStaticMarkup(<DocMarkdown text={text} />)
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
        <Button size="sm" variant={view === 'rich' ? 'secondary' : 'ghost'} aria-pressed={view === 'rich'} onClick={() => setView('rich')}>
          Rich text
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
            hosted()
              ? { label: 'PDF', hint: "through this browser's print dialog", onSelect: () => void page().then(printInBrowser) }
              : { label: 'PDF', hint: 'printed on this computer', onSelect: () => void pdf() },
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
      ) : view === 'rich' ? (
        <div className="min-h-0 flex-1 overflow-auto">
          <article className="mx-auto max-w-3xl px-6 pt-6 pb-10">
            <RichDoc text={text} onChange={typed} editable />
          </article>
          <DocComments thread={thread} comments={commentsHere} onShow={show} onChanged={refreshComments} />
        </div>
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
          <article ref={reading} className="mx-auto max-w-3xl px-6 pt-6 pb-10">
            <DocMarkdown text={text} />
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
  // The sheet shown, from 0; an artifact has up to 16 (ADR 0077).
  const [tab, setTab] = useState(0)
  const key = ['sheet', thread, artifact, tab]
  const sheet = useQuery({
    queryKey: key,
    queryFn: () => api.sheetData(thread, artifact, { op: 'read', sheet: tab }) as Promise<SheetView>,
    refetchInterval: 5_000,
  })
  const [editing, setEditing] = useState<{ row: number; column: number; raw: string } | null>(null)
  const [selected, setSelected] = useState<{ row: number; column: number }>({ row: 0, column: 0 })
  const [renaming, setRenaming] = useState<string | null>(null)
  // Comments on this sheet's cells, by where each cell is now (ADR 0082).
  const comments = useQuery({ queryKey: keys.comments(thread), queryFn: () => api.comments(thread) })
  const [commenting, setCommenting] = useState<string | null>(null)
  const refreshComments = () => queries.invalidateQueries({ queryKey: keys.comments(thread) })
  const onCells = new Map<string, CommentView[]>()
  for (const comment of comments.data ?? []) {
    const at = comment.location
    if (at.at !== 'cell' || at.artifact !== artifact || at.sheet !== tab || at.outdated) continue
    onCells.set(at.address, [...(onCells.get(at.address) ?? []), comment])
  }
  const addComment = async (address: string) => {
    if (!commenting?.trim()) return
    try {
      await api.addComment(thread, { on: 'cell', artifact, sheet: tab, address }, commenting.trim())
      setCommenting(null)
      await refreshComments()
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  // A column being made wider or narrower by its header's edge, until it's let go.
  const [sizing, setSizing] = useState<{ column: number; width: number; from: number; start: number } | null>(null)
  // Only the rows near what's shown are drawn, so a sheet of thousands of rows stays quick (ADR 0077).
  const scroller = useRef<HTMLDivElement>(null)
  const [window_, setWindow] = useState({ top: 0, height: 800 })
  const measure = () => {
    const box = scroller.current
    if (box) setWindow({ top: box.scrollTop, height: box.clientHeight })
  }
  const change = async (asked: Record<string, unknown>) => {
    try {
      await api.sheetData(thread, artifact, { sheet: tab, ...asked })
      await queries.invalidateQueries({ queryKey: ['sheet', thread, artifact] })
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  const commit = async () => {
    if (!editing) return
    const at = `${columnName(editing.column)}${editing.row + 1}`
    setEditing(null)
    // By the row and column the person typed in, so one put in above meanwhile doesn't move it.
    const ids = view && view.rows[editing.row] && view.columns[editing.column] ? { [at]: [view.rows[editing.row], view.columns[editing.column]] } : {}
    await change({ op: 'set', cells: { [at]: editing.raw }, ids })
  }
  // The sheet shown, its values as they show, a cell that a spreadsheet app would run quoted first.
  const csv = () => {
    if (!view) return
    const safe = (value: string) => (/^[=+\-@\t\r]/.test(value) && Number.isNaN(Number(value)) ? `'${value}` : value)
    const text = csvOf(
      view.columns.map((_id, column) => columnName(column)),
      view.values.map((row) => row.map(safe)),
    )
    const url = URL.createObjectURL(new Blob([text], { type: 'text/csv' }))
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = `${(view.names?.[tab] ?? title).replace(/[^A-Za-z0-9_-]+/g, '-') || 'sheet'}.csv`
    anchor.click()
    setTimeout(() => URL.revokeObjectURL(url), 1000)
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
  const selectedAddress = `${columnName(selected.column)}${selected.row + 1}`
  const selectedComments = onCells.get(selectedAddress) ?? []
  const ROW = 29
  const windowed = view.rows.length > 200
  const first = windowed ? Math.max(0, Math.floor(window_.top / ROW) - 20) : 0
  const last = windowed ? Math.min(view.rows.length, Math.ceil((window_.top + window_.height) / ROW) + 20) : view.rows.length
  const drawn = view.rows.slice(first, last).map((id, at) => [id, first + at] as const)
  const widthOf = (column: number) => (sizing?.column === column ? sizing.width : (view.widths?.[column] ?? null))
  const sized = (column: number) => {
    const width = widthOf(column)
    return width ? { width, minWidth: width, maxWidth: width } : undefined
  }
  const rename = async () => {
    if (renaming === null) return
    const name = renaming.trim()
    setRenaming(null)
    if (name !== (view.names?.[tab] ?? '')) await change({ op: 'format', name: name === `Sheet ${tab + 1}` ? '' : name })
  }
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
        <select
          aria-label="number format of the selected column"
          value={view.formats?.[selected.column] ?? ''}
          onChange={(event) => void change({ op: 'format', formats: { [columnName(selected.column)]: event.target.value || null } })}
          className="h-7 rounded-sm border border-line bg-surface-1 px-1 text-xs text-fg-1"
        >
          {NUMBER_FORMATS.map(([format, label]) => (
            <option key={format} value={format}>
              {label}
            </option>
          ))}
        </select>
        <select
          aria-label="number format of the selected cell"
          value={view.cell_formats?.[`${columnName(selected.column)}${selected.row + 1}`] ?? ''}
          onChange={(event) =>
            void change({ op: 'format', cells: { [`${columnName(selected.column)}${selected.row + 1}`]: event.target.value || null } })
          }
          className="h-7 rounded-sm border border-line bg-surface-1 px-1 text-xs text-fg-1"
        >
          {NUMBER_FORMATS.map(([format, label]) => (
            <option key={format} value={format}>
              {format === '' ? 'As its column' : label}
            </option>
          ))}
        </select>
        <Button size="sm" variant="ghost" onClick={() => void change({ op: 'insert', axis: 'rows', at: selected.row })}>
          Row above
        </Button>
        <Button size="sm" variant="ghost" onClick={() => void change({ op: 'insert', axis: 'columns', at: selected.column })}>
          Column left
        </Button>
        <Button size="sm" variant="ghost" onClick={() => setCommenting(commenting === null ? '' : null)}>
          Comment
        </Button>
        <Button size="sm" variant="ghost" onClick={csv}>
          CSV
        </Button>
        <IconButton icon="download" label="Download as .xlsx" onClick={() => void xlsx()} />
      </div>
      {/* Focusable, so the grid scrolls by keyboard too. */}
      <div className="min-h-0 flex-1 overflow-auto" ref={scroller} onScroll={windowed ? measure : undefined} tabIndex={0} aria-label={`${title}'s cells`}>
        <table className="border-separate border-spacing-0 font-mono text-xs" role="grid" aria-label={title}>
          <thead className="sticky top-0 z-10 bg-surface-1">
            <tr>
              <th className="w-10 border-r border-b border-line" />
              {view.columns.map((id, column) => (
                <th
                  key={id}
                  scope="col"
                  style={sized(column)}
                  className={cx('relative border-r border-b border-line px-2 py-1 font-medium text-fg-2', !widthOf(column) && 'min-w-24')}
                >
                  {columnName(column)}
                  <span
                    role="separator"
                    aria-orientation="vertical"
                    aria-label={`width of column ${columnName(column)}`}
                    className="absolute inset-y-0 right-0 w-1.5 cursor-col-resize hover:bg-accent/40"
                    onPointerDown={(event) => {
                      event.currentTarget.setPointerCapture(event.pointerId)
                      const start = event.currentTarget.parentElement?.getBoundingClientRect().width ?? 96
                      setSizing({ column, width: Math.round(start), from: event.clientX, start: Math.round(start) })
                    }}
                    onPointerMove={(event) => {
                      if (sizing?.column === column) setSizing({ ...sizing, width: Math.min(800, Math.max(24, Math.round(sizing.start + event.clientX - sizing.from))) })
                    }}
                    onPointerUp={() => {
                      if (sizing?.column !== column) return
                      setSizing(null)
                      if (sizing.width !== sizing.start) void change({ op: 'format', widths: { [columnName(column)]: sizing.width } })
                    }}
                  />
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {first > 0 && <tr aria-hidden style={{ height: first * ROW }} />}
            {drawn.map(([id, row]) => (
              <tr key={id}>
                <th scope="row" className="sticky left-0 border-r border-b border-line bg-surface-1 px-2 text-right font-medium text-fg-2">
                  {row + 1}
                </th>
                {view.columns.map((columnId, column) => {
                  const active = selected.row === row && selected.column === column
                  const value = view.values[row]?.[column] ?? ''
                  const commented = onCells.get(`${columnName(column)}${row + 1}`)
                  return (
                    <td
                      key={columnId}
                      style={sized(column)}
                      className={cx('relative h-7 border-r border-b border-line-subtle p-0', active && 'outline-2 -outline-offset-2 outline-accent')}
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
                          {commented && (
                            <span
                              aria-label={`${commented.length} comment${commented.length === 1 ? '' : 's'}`}
                              className="absolute top-0 right-0 h-0 w-0 border-t-[6px] border-l-[6px] border-t-accent border-l-transparent"
                            />
                          )}
                        </span>
                      )}
                    </td>
                  )
                })}
              </tr>
            ))}
            {last < view.rows.length && <tr aria-hidden style={{ height: (view.rows.length - last) * ROW }} />}
          </tbody>
        </table>
      </div>
      {(commenting !== null || selectedComments.length > 0) && (
        <section aria-label={`comments on ${selectedAddress}`} data-testid="cell-comments" className="flex max-h-48 shrink-0 flex-col gap-2 overflow-auto border-t border-line-subtle px-3 py-2">
          {selectedComments.map((comment) => (
            <div key={comment.id} className={cx('flex flex-col gap-1', comment.resolved && 'opacity-60')}>
              <CommentThread thread={thread} comment={comment} onChanged={refreshComments} />
              {!comment.resolved && (
                <Button variant="ghost" size="sm" className="self-start" onClick={() => void api.resolveComment(thread, comment.id).then(refreshComments)}>
                  Resolve
                </Button>
              )}
            </div>
          ))}
          {commenting !== null && (
            <form
              className="flex items-center gap-2"
              onSubmit={(event) => {
                event.preventDefault()
                void addComment(selectedAddress)
              }}
            >
              <Input autoFocus aria-label={`a comment on ${selectedAddress}`} value={commenting} onChange={(event) => setCommenting(event.target.value)} placeholder={`Comment on ${selectedAddress}`} />
              <Button size="sm" type="submit" disabled={!commenting.trim()}>
                Add
              </Button>
            </form>
          )}
        </section>
      )}
      <div className="flex shrink-0 items-center gap-1 overflow-x-auto border-t border-line-subtle px-2 py-1">
        <div className="flex items-center gap-1" role="tablist" aria-label="sheets">
        {Array.from({ length: view.sheets ?? 1 }, (_, at) =>
          tab === at && renaming !== null ? (
            <input
              key={at}
              autoFocus
              aria-label="the sheet's name"
              value={renaming}
              maxLength={64}
              onChange={(event) => setRenaming(event.target.value)}
              onBlur={() => void rename()}
              onKeyDown={(event) => {
                if (event.key === 'Enter') void rename()
                if (event.key === 'Escape') setRenaming(null)
              }}
              className="w-32 rounded bg-bg px-2 py-0.5 text-xs text-fg-1 outline-1 outline-accent"
            />
          ) : (
            <button
              key={at}
              role="tab"
              aria-selected={tab === at}
              title={tab === at ? 'Double-click to rename' : undefined}
              onClick={() => {
                setTab(at)
                setSelected({ row: 0, column: 0 })
                setEditing(null)
              }}
              onDoubleClick={() => tab === at && setRenaming(view.names?.[at] ?? `Sheet ${at + 1}`)}
              className={cx('rounded px-2 py-0.5 text-xs', tab === at ? 'bg-surface-2 text-fg-1' : 'text-fg-2 hover:text-fg-1')}
            >
              {view.names?.[at] ?? `Sheet ${at + 1}`}
            </button>
          ),
        )}
        </div>
        {(view.sheets ?? 1) < 16 && (
          <Button
            size="sm"
            variant="ghost"
            onClick={async () => {
              const next = view.sheets ?? 1
              try {
                await api.sheetData(thread, artifact, { op: 'set', sheet: next, cells: { E10: '' } })
                await queries.invalidateQueries({ queryKey: ['sheet', thread, artifact] })
                setTab(next)
              } catch (failure) {
                toast(message(failure), 'danger')
              }
            }}
          >
            Add a sheet
          </Button>
        )}
      </div>
    </div>
  )
}

/** The number formats a column may take, as the sheet's engine shows them (ADR 0083). */
const NUMBER_FORMATS: [string, string][] = [
  ['', 'General'],
  ['0', '1234'],
  ['0.00', '1234.10'],
  ['#,##0', '1,234'],
  ['#,##0.00', '1,234.10'],
  ['0%', '12%'],
  ['0.0%', '12.3%'],
  ['$#,##0.00', '$1,234.10'],
  ['€#,##0.00', '€1,234.10'],
  ['£#,##0.00', '£1,234.10'],
  ['yyyy-mm-dd', '2026-11-16'],
  ['d mmm yyyy', '16 Nov 2026'],
  ['dd/mm/yyyy', '16/11/2026'],
  ['mm/dd/yyyy', '11/16/2026'],
  ['mmm yyyy', 'Nov 2026'],
]
