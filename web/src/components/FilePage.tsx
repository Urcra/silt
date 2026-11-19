import { useQuery } from '@tanstack/react-query'
import { Link, useParams, useRouterState } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { api, short } from '../api'
import { filePath, type LineRange, linesHash, parseLines } from '../permalink'
import { keys, message } from '../query'
import { CopyLink } from './CopyLink'
import { Page } from './Page'

/** A file of a thread at a version, with lines to link to (ADR 0031). */
export function FilePage() {
  const { id, _splat: path = '' } = useParams({ from: '/t/$id/files/$' })
  const location = useRouterState({ select: (state) => state.location })
  const at = new URLSearchParams(location.searchStr).get('at') ?? undefined
  const file = useQuery({ queryKey: keys.fileAt(id, path, at), queryFn: () => api.fileAt(id, path, at), retry: false })
  const [lines, setLines] = useState<LineRange | null>(() => parseLines(location.hash))
  const anchor = useRef<number | null>(lines?.start ?? null)
  const scrolled = useRef(false)

  useEffect(() => {
    if (!file.data || !lines || scrolled.current) return
    scrolled.current = true
    document.getElementById(`L${lines.start}`)?.scrollIntoView({ block: 'center' })
  }, [file.data, lines])

  const pick = (line: number, extend: boolean) => {
    const picked = extend && anchor.current !== null ? parseLines(`L${anchor.current}-L${line}`) : { start: line, end: line }
    if (!extend) anchor.current = line
    setLines(picked)
    // The page's <base> would take a bare `#L4` to the app's root.
    const { pathname, search } = window.location
    if (picked) window.history.replaceState(window.history.state, '', `${pathname}${search}#${linesHash(picked)}`)
  }

  return (
    <div>
      <p className="text-sm">
        <Link className="text-accent-fg" to="/t/$id" params={{ id }}>
          thread {short(id)}
        </Link>
      </p>
      <div className="my-2 flex flex-wrap items-baseline gap-3">
        <h2 className="font-mono text-lg font-semibold">{path}</h2>
        {file.data && (
          <span className="font-mono text-xs text-fg-3" title={file.data.at}>
            {at ? 'at a pinned version' : 'as the thread has it now'}
          </span>
        )}
        {file.data && (
          <CopyLink
            path={filePath(id, path, file.data.at, lines ?? undefined)}
            thread={id}
            label={lines ? 'copy link to these lines' : 'copy link'}
          />
        )}
      </div>
      {file.error && <p className="text-danger">{message(file.error)}</p>}
      {file.data?.text === null && <p className="text-sm text-fg-2">A binary file.</p>}
      {file.data?.text != null && (
        <pre data-testid="file" className="overflow-x-auto rounded-md border border-line font-mono text-xs leading-5">
          {file.data.text
            .replace(/\n$/, '')
            .split('\n')
            .map((text, index) => {
              const line = index + 1
              const picked = lines !== null && line >= lines.start && line <= lines.end
              return (
                <div
                  key={line}
                  id={`L${line}`}
                  data-picked={picked || undefined}
                  className={`flex ${picked ? 'bg-accent-tint' : ''}`}
                >
                  <button
                    type="button"
                    aria-label={`line ${line}`}
                    className="w-12 shrink-0 select-none pr-2 text-right text-fg-3 hover:text-accent-fg"
                    onClick={(event) => pick(line, event.shiftKey)}
                  >
                    {line}
                  </button>
                  <span className="whitespace-pre pr-2">{text || ' '}</span>
                </div>
              )
            })}
        </pre>
      )}
    </div>
  )
}

/** FilePage in a plain page of the shell, until it's drawn as its board (ADR 0039). */
export function FileRoute() {
  return (
    <Page width="wide">
      <FilePage />
    </Page>
  )
}
