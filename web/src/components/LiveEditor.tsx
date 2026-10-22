import { EditorView, basicSetup } from 'codemirror'
import { useEffect, useRef, useState } from 'react'
import type { LiveReplica } from '../replica'

const EDIT_DEBOUNCE_MS = 150

/** Puts the cursor on `line` and scrolls it into the middle. */
function showLine(view: EditorView, line: number) {
  const at = view.state.doc.line(Math.min(Math.max(line, 1), view.state.doc.lines)).from
  view.dispatch({ selection: { anchor: at }, effects: EditorView.scrollIntoView(at, { y: 'center' }) })
}

/** Applies `next` to the editor as the smallest change, so cursors keep their place. */
function replaceText(view: EditorView, next: string) {
  const current = view.state.doc.toString()
  if (current === next) return
  let start = 0
  while (start < current.length && start < next.length && current[start] === next[start]) start++
  let end = 0
  while (
    end < current.length - start &&
    end < next.length - start &&
    current[current.length - 1 - end] === next[next.length - 1 - end]
  )
    end++
  view.dispatch({ changes: { from: start, to: current.length - end, insert: next.slice(start, next.length - end) } })
}

/** A file of the live replica in an editor: a module of its own, as the
 * editor is most of the thread page's code and opening a file is rarer than
 * reading the thread. */
export function FileEditor({
  replica,
  path,
  changes,
  onLine,
  reveal,
  onType,
}: {
  replica: LiveReplica
  path: string
  changes: number
  /** Where this viewer's cursor moved, as a line from 1. */
  onLine: (line: number) => void
  /** A line to show, from someone being followed. */
  reveal: number | null
  /** This viewer typed, which ends following someone. */
  onType: () => void
}) {
  const lineChanged = useRef(onLine)
  const typed = useRef(onType)
  const revealed = useRef(reveal)
  useEffect(() => {
    lineChanged.current = onLine
    typed.current = onType
    revealed.current = reveal
  })
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null)
  const applying = useRef(false)
  const [error, setError] = useState<string>()

  const flush = async () => {
    if (pending.current) clearTimeout(pending.current)
    pending.current = null
    if (view.current) await replica.edit(path, view.current.state.doc.toString()).catch((e: Error) => setError(e.message))
  }

  useEffect(() => {
    let current = true
    void replica.show(path).then((text) => {
      if (!current || !host.current) return
      view.current = new EditorView({
        doc: text ?? '',
        parent: host.current,
        extensions: [
          basicSetup,
          EditorView.editable.of(replica.writable),
          EditorView.domEventHandlers({ keydown: () => typed.current() }),
          EditorView.updateListener.of((update) => {
            if (update.selectionSet || update.docChanged) {
              lineChanged.current(update.state.doc.lineAt(update.state.selection.main.head).number)
            }
            if (!update.docChanged || applying.current) return
            if (pending.current) clearTimeout(pending.current)
            pending.current = setTimeout(() => void flush(), EDIT_DEBOUNCE_MS)
          }),
        ],
      })
      if (revealed.current !== null) showLine(view.current, revealed.current)
    })
    return () => {
      current = false
      view.current?.destroy()
      view.current = null
    }
    // The editor is made once per file; later text arrives through `changes`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [replica, path])

  useEffect(() => {
    if (changes === 0) return
    void (async () => {
      await flush()
      await replica.sync()
      await replica.prepare(path)
      if (!view.current) return
      // From here on nothing awaits, so no keystroke can land between taking
      // the editor's text and replacing it with the merged one.
      if (pending.current) clearTimeout(pending.current)
      pending.current = null
      const sent = replica.edit(path, view.current.state.doc.toString())
      const text = replica.showNow(path)
      if (text !== undefined) {
        applying.current = true
        replaceText(view.current, text)
        applying.current = false
      }
      await sent.catch((e: Error) => setError(e.message))
    })()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [changes])

  useEffect(() => {
    if (view.current && reveal !== null) showLine(view.current, reveal)
  }, [reveal, path])

  return (
    <div>
      {!replica.writable && <p className="text-xs text-fg-3">Read only: ask the owner to share the thread to edit it.</p>}
      {error && <p className="text-xs text-danger">{error}</p>}
      <div ref={host} data-testid="editor" className="rounded border border-line text-sm" />
    </div>
  )
}
