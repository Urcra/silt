// What agents showed the person (ADR 0068): images in the conversation,
// artifact cards there, and the panel that shows an artifact beside it.

import { useQuery } from '@tanstack/react-query'
import { useEffect, useId, useRef, useState, type ReactNode } from 'react'
import { api, type ArtifactContent, type ArtifactView, type CommentView } from '../api'
import {
  type ArtifactGroup,
  type ArtifactVersion,
  type FrameProblem,
  type FrameTheme,
  dataUrl,
  fileName,
  kindLabel,
  latest,
  offlineHtml,
  problemsPrompt,
  sandboxedDocument,
  svgUrl,
  textOf,
  versionOf,
} from '../artifacts'
import type { Opened } from '../artifactActions'
import { keys, message } from '../query'
import { useNow } from '../now'
import { usePermalink } from '../usePermalink'
import { Links } from './ShareDialog'
import { ago } from '../time'
import { useShownTheme } from '../theme'
import { Button, Dialog, Icon, IconButton, Menu, type MenuItem, Spinner, cx, toast } from '../ui'
import { ArtifactCommentsButton, AskForChange } from './ArtifactComments'
import { KindIcon, ShownImage } from './ArtifactCards'
import { Markdown } from './Markdown'
import { mermaidSvg, withDiagramsDrawn } from '../mermaid'
import { Broker, callOf } from '../bridge/broker'
import { handlersFor } from '../bridge/capabilities'
import { lastSnapshot, rememberSnapshot } from '../bridge/hot'
import { keep, keptFor } from '../bridge/storage'

/** The Artifacts tab with none open: the thread's artifacts, the one changed last first, and the images agents showed. */
export function ArtifactList({
  thread,
  groups,
  images,
  onOpen,
}: {
  thread: string
  groups: ArtifactGroup[]
  images: ArtifactVersion[]
  onOpen: (id: string) => void
}) {
  const now = useNow()
  if (groups.length === 0 && images.length === 0)
    return (
      <p className="px-5 py-6 text-sm text-fg-2">
        Nothing yet. When an agent you run here makes a page, a chart or a document for you, it shows here, and images it shows you are kept here too.
      </p>
    )
  return (
    <div className="flex flex-col gap-5 px-4 py-4" data-testid="artifact-list">
      {groups.length > 0 && (
        <section aria-label="artifacts" className="flex flex-col gap-2">
          <h3 className="font-mono text-2xs tracking-[0.08em] text-fg-2 uppercase">Artifacts</h3>
          {groups.map((group) => {
            const last = latest(group)
            return (
              <button
                key={group.id}
                type="button"
                onClick={() => onOpen(group.id)}
                className="flex items-center gap-3 rounded-md border border-line bg-elevated p-2.5 text-left hover:border-line-strong"
              >
                <span className="flex h-12 w-12 shrink-0 items-center justify-center rounded-sm bg-surface-1 text-fg-2">
                  <KindIcon kind={last.artifact.kind} className="h-5 w-5" />
                </span>
                <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="truncate font-medium">{last.artifact.title}</span>
                  <span className="text-xs text-fg-2">
                    {kindLabel(last.artifact.kind)} · {group.versions.length === 1 ? '1 version' : `${group.versions.length} versions`}
                  </span>
                  <span className="font-mono text-2xs text-fg-3">{ago(last.entry.time_ms, now)}</span>
                </span>
                <Icon name="chevronRight" className="h-3.5 w-3.5 text-fg-2" />
              </button>
            )
          })}
        </section>
      )}
      {images.length > 0 && (
        <section aria-label="images the agent showed" className="flex flex-col gap-2">
          <h3 className="font-mono text-2xs tracking-[0.08em] text-fg-2 uppercase">Images the agent showed</h3>
          <div className="grid grid-cols-2 gap-3 md:grid-cols-3">
            {images.map((image) => (
              <ShownImage key={image.entry.index} thread={thread} entry={image.entry} />
            ))}
          </div>
        </section>
      )}
    </div>
  )
}

const ZOOMS = [50, 75, 100, 125, 150, 200]
const PRESSED = 'aria-pressed:bg-surface-2 aria-pressed:text-fg-1'
const ICON_TRIGGER = 'inline-flex items-center justify-center rounded-sm p-1 text-fg-2 hover:bg-surface-2 hover:text-fg-1 max-md:min-h-11 max-md:min-w-11'

/**
 * An open artifact (ADR 0068, ADR 0070): one header row holds its title, version and view, and its page fills the rest
 * edge to edge. As the panel narrows, the view's controls fold into ⋯.
 */
export function ArtifactViewer({
  thread,
  groups,
  opened,
  onOpen,
  onBack,
  onClose,
  full,
  onFull,
  comments,
  onCommentsChanged,
  phone = false,
}: {
  thread: string
  groups: ArtifactGroup[]
  opened: Opened
  onOpen: (id: string, version: number | null) => void
  onBack: () => void
  onClose: () => void
  /** Whether it takes the conversation's space too; absent where there is no such view, as on a phone. */
  full?: boolean
  onFull?: () => void
  /** The thread's comments, among them those on this artifact's versions. */
  comments: CommentView[]
  onCommentsChanged: () => Promise<unknown>
  /** On a phone: a box under the page asks the agent for a change. */
  phone?: boolean
}) {
  const group = groups.find((candidate) => candidate.id === opened.id)
  const [source, setSource] = useState(false)
  const [zoom, setZoom] = useState(100)
  const [reloads, setReloads] = useState(0)
  // What's selected in the page, for a comment to quote.
  const [quote, setQuote] = useState<string | null>(null)
  const [sharing, setSharing] = useState(false)
  const shown = group ? versionOf(group, opened.version) : null
  const content = useQuery({
    queryKey: keys.artifact(thread, shown?.artifact.blob ?? ''),
    queryFn: () => api.artifact(thread, shown!.artifact.blob),
    enabled: Boolean(shown),
    staleTime: Infinity,
  })
  // The CSS the version's design system compiled to, which its frame is given (ADR 0071).
  const styleBlob = shown?.artifact.style ?? null
  const style = useQuery({
    queryKey: keys.artifact(thread, styleBlob ?? ''),
    queryFn: () => api.artifact(thread, styleBlob!),
    enabled: Boolean(styleBlob),
    staleTime: Infinity,
  })
  // Libraries and fonts come through Silt's vendor route when that's on (ADR 0071).
  const settings = useQuery({ queryKey: keys.artifactSettings, queryFn: api.artifactSettings, staleTime: 60_000 })
  const vendor = settings.data?.proxy ? `${window.location.origin}/vendor/` : null
  // What this viewer's frame saw go wrong in the version shown, until it's sent or dismissed.
  const [problems, setProblems] = useState<{ blob: string; seen: FrameProblem[] } | null>(null)
  const link = usePermalink(thread)
  const now = useNow()
  if (!group || !shown) return <p className="px-5 py-6 text-sm text-fg-2">This artifact isn't in the thread's history.</p>
  const artifact = shown.artifact
  const styleText = style.data ? textOf(style.data) : null
  const styled = (!styleBlob || style.isSuccess || style.isError) && (settings.isSuccess || settings.isError)
  const seen = problems?.blob === artifact.blob ? problems.seen : []
  const sendProblems = async () => {
    try {
      await api.prompt(thread, problemsPrompt(shown, seen))
      setProblems(null)
      toast('Sent to the agent')
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  const textual = artifact.kind !== 'image'
  const download = (bytes: BlobPart, type: string, name: string) => {
    const url = URL.createObjectURL(new Blob([bytes], { type }))
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = name
    anchor.click()
    setTimeout(() => URL.revokeObjectURL(url), 1000)
  }
  const downloadFile = () => {
    if (!content.data) return
    const bytes = Uint8Array.from(atob(content.data.data), (char) => char.charCodeAt(0))
    download(bytes, content.data.media_type, fileName(artifact))
  }
  const exportHtml = async () => {
    if (!content.data) return
    try {
      const rendered = await renderedMarkup(artifact, content.data)
      download(offlineHtml(artifact, content.data, rendered, styleText), 'text/html', fileName(artifact, 'html'))
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  const latestVersion = latest(group).artifact.version
  const zoomTo = (step: number) => ZOOMS[ZOOMS.indexOf(zoom) + step]
  const shareItems: MenuItem[] = [
    { label: 'Read-only link…', icon: 'lock', hint: 'no account needed', onSelect: () => setSharing(true) },
    { label: 'Copy link', icon: 'link', hint: 'for people in the thread', onSelect: () => link(`/t/${thread}#artifact-${artifact.id}`) },
    { label: 'Download as one HTML file', icon: 'download', hint: 'opens anywhere', onSelect: () => void exportHtml(), divided: true },
    { label: 'Download the file', icon: 'file', onSelect: downloadFile },
  ]
  const viewItems: MenuItem[] = [
    { label: 'Preview', icon: source ? 'eye' : 'check', onSelect: () => setSource(false) },
    ...(textual ? [{ label: 'Source', icon: source ? 'check' : 'code', onSelect: () => setSource(true) } satisfies MenuItem] : []),
  ]
  const zoomItem = (label: string, step: number): MenuItem => {
    const level = zoomTo(step)
    return { label, hint: level ? `to ${level}%` : undefined, disabled: !level, onSelect: () => setZoom(level ?? zoom) }
  }
  const pageItems: MenuItem[] = [
    { label: 'Reload', icon: 'reload', onSelect: () => setReloads(reloads + 1) },
    zoomItem('Zoom in', 1),
    zoomItem('Zoom out', -1),
  ]
  // On a phone the menu opens from near the screen's right edge, so the share items' hints would push it off the left.
  const narrowItems: MenuItem[] = [
    ...viewItems,
    ...pageItems.map((item, at) => (at === 0 ? { ...item, divided: true } : item)),
    ...shareItems.map((item, at) => ({ ...item, hint: undefined, divided: at === 0 || item.divided })),
  ]
  const more = (items: MenuItem[]) => <Menu label="More" trigger={<Icon name="more" className="h-4 w-4" />} triggerClassName={ICON_TRIGGER} items={items} />
  return (
    <div className="@container/artifact flex h-full min-h-0 flex-col" data-testid="artifact-viewer">
      <div className="sticky top-0 z-20 flex h-12 shrink-0 items-center gap-0.5 border-b border-line-subtle bg-bg pr-2 pl-1.5">
        <button
          type="button"
          onClick={onBack}
          title="Changes, Artifacts, Details, Lineage"
          className="inline-flex shrink-0 items-center gap-1 rounded-sm px-1.5 py-1 text-sm text-fg-2 hover:bg-surface-2 hover:text-fg-1 max-md:min-h-11"
        >
          <Icon name="chevronLeft" className="h-3.5 w-3.5" />
          <span className="@max-md/artifact:sr-only">Artifacts</span>
        </button>
        <span aria-hidden="true" className="text-fg-3">
          /
        </span>
        <span className="flex min-w-0 *:min-w-0">
          <Menu
            label="Pick an artifact"
            align="left"
            trigger={
              <>
                <span className="truncate">{artifact.title}</span>
                <Icon name="chevronDown" className="h-3 w-3 shrink-0 text-fg-2" />
              </>
            }
            triggerClassName="inline-flex max-w-full min-w-0 items-center gap-1.5 rounded-sm px-1.5 py-1 text-sm font-medium hover:bg-surface-2 max-md:min-h-11"
            items={groups.map((candidate) => ({
              label: latest(candidate).artifact.title,
              hint: `${kindLabel(latest(candidate).artifact.kind)} · v${latest(candidate).artifact.version}`,
              icon: candidate.id === group.id ? 'check' : undefined,
              onSelect: () => onOpen(candidate.id, null),
            }))}
          />
        </span>
        <span className="shrink-0">
          <Menu
            label="Versions"
            align="left"
            trigger={
              <>
                v{artifact.version}
                <Icon name="chevronDown" className="h-3 w-3" />
              </>
            }
            triggerClassName="inline-flex items-center gap-1 rounded-sm border border-line px-1.5 py-0.5 font-mono text-xs text-fg-2 hover:border-line-strong hover:text-fg-1 max-md:min-h-11"
            items={[...group.versions].reverse().map((version) => ({
              label: `v${version.artifact.version}${version.artifact.version === latestVersion ? ' · latest' : ''}`,
              hint: [version.artifact.note, ago(version.entry.time_ms, now)].filter(Boolean).join(' · '),
              icon: version.artifact.version === artifact.version ? 'check' : undefined,
              onSelect: () => onOpen(group.id, version.artifact.version === latestVersion ? null : version.artifact.version),
            }))}
          />
        </span>
        <span className="min-w-2 flex-1" />
        <span role="group" aria-label="View" className="mr-1 inline-flex shrink-0 gap-px rounded-sm bg-surface-1 p-0.5 @max-md/artifact:hidden">
          <IconButton icon="eye" label="Preview" aria-pressed={!source} onClick={() => setSource(false)} className={PRESSED} />
          {textual && <IconButton icon="code" label="Source" aria-pressed={source} onClick={() => setSource(true)} className={PRESSED} />}
        </span>
        <span className="inline-flex shrink-0 items-center gap-0.5 @max-xl/artifact:hidden">
          <IconButton icon="reload" label="Reload" onClick={() => setReloads(reloads + 1)} />
          <Menu
            label="Zoom"
            trigger={
              <>
                {zoom}%
                <Icon name="chevronDown" className="h-3 w-3 text-fg-2" />
              </>
            }
            triggerClassName="inline-flex items-center gap-1 rounded-sm px-1.5 py-1 text-xs text-fg-1 hover:bg-surface-2 max-md:min-h-11"
            items={ZOOMS.map((level) => ({ label: `${level}%`, icon: level === zoom ? 'check' : undefined, onSelect: () => setZoom(level) }))}
          />
          <span aria-hidden="true" className="mx-1 h-4 w-px bg-line" />
        </span>
        <span className="shrink-0 @max-md/artifact:hidden">
          <Menu
            label="Share"
            trigger={
              <>
                <Icon name="lock" className="h-4 w-4" />
                <span className="max-md:sr-only @max-xl/artifact:sr-only">Share</span>
              </>
            }
            triggerClassName="inline-flex items-center gap-1.5 rounded-sm px-2 py-1 text-sm text-fg-1 hover:bg-surface-2 max-md:min-h-11"
            items={shareItems}
          />
        </span>
        <span className="hidden shrink-0 @md/artifact:@max-xl/artifact:inline-flex">{more(pageItems)}</span>
        <span className="hidden shrink-0 @max-md/artifact:inline-flex">
          {more(narrowItems)}
        </span>
        <ArtifactCommentsButton
          thread={thread}
          group={group}
          shown={shown}
          comments={comments}
          quote={quote}
          onClearQuote={() => setQuote(null)}
          onChanged={onCommentsChanged}
        />
        {onFull && (
          <IconButton icon={full ? 'collapse' : 'expand'} label={full ? 'Leave the full view (F)' : 'Full view (F)'} onClick={onFull} className="max-lg:hidden" />
        )}
        <IconButton icon="x" label="Close the artifact" onClick={onClose} />
      </div>
      <div className="relative min-h-0 flex-1 overflow-hidden bg-elevated">
        {content.data && styled ? (
          <ArtifactPage
            key={`${artifact.blob}-${reloads}`}
            thread={thread}
            artifact={artifact}
            content={content.data}
            style={styleText}
            vendor={vendor}
            source={source}
            zoom={zoom}
            full={Boolean(full)}
            onSelect={(text) => setQuote(text || null)}
            onProblems={(found) => setProblems({ blob: artifact.blob, seen: found })}
          />
        ) : content.error ? (
          <p className="px-5 py-6 text-sm text-danger">{message(content.error)}</p>
        ) : (
          <div className="flex justify-center pt-20">
            <Spinner />
          </div>
        )}
        {seen.length > 0 && !source && (
          <div
            role="status"
            className="absolute inset-x-2.5 bottom-2.5 z-10 flex flex-wrap items-center gap-2 rounded-md border border-line bg-elevated px-3 py-2 text-xs text-fg-1 shadow-overlay"
            data-testid="artifact-problems"
          >
            <Icon name="alert" className="h-3.5 w-3.5 shrink-0 text-warn" />
            <span className="min-w-0 flex-1">
              {seen.length === 1 ? 'Your browser saw a problem in this version: ' : `Your browser saw ${seen.length} problems in this version, first: `}
              <span className="font-mono break-all text-fg-2">{seen[0].text}</span>
            </span>
            <Button size="sm" onClick={() => void sendProblems()}>
              Send to the agent
            </Button>
            <IconButton icon="x" label="Dismiss the problems" onClick={() => setProblems(null)} />
          </div>
        )}
      </div>
      <Dialog open={sharing} onClose={() => setSharing(false)} title={`Share ${artifact.title}`}>
        {sharing && <Links thread={thread} artifact={{ id: artifact.id, version: artifact.version }} />}
      </Dialog>
      {phone && (
        <AskForChange
          thread={thread}
          group={group}
          shown={shown}
          comments={comments}
          quote={quote}
          onClearQuote={() => setQuote(null)}
          onChanged={onCommentsChanged}
        />
      )}
    </div>
  )
}

function isProblem(value: unknown): value is FrameProblem {
  const problem = value as FrameProblem | null
  return Boolean(problem) && ['blocked', 'error', 'overflow'].includes(problem!.kind) && typeof problem!.text === 'string'
}

/** The page itself: an HTML artifact in a frame that can reach nothing, anything else drawn here without running it. */
function ArtifactPage({
  thread,
  artifact,
  content,
  style,
  vendor,
  source,
  zoom,
  full,
  onSelect,
  onProblems,
}: {
  thread: string
  artifact: ArtifactView
  content: ArtifactContent
  style: string | null
  vendor: string | null
  source: boolean
  zoom: number
  full: boolean
  onSelect: (text: string) => void
  onProblems: (problems: FrameProblem[]) => void
}) {
  const frame = useRef<HTMLIFrameElement>(null)
  const theme = useShownTheme()
  // The frame starts in the theme shown when it loads, and hears of each change after, so its page isn't reloaded.
  const [startTheme] = useState(theme)
  // An HTML version loads from a frame address the daemon or server composes it at (ADR 0074). The key behind it
  // lasts an hour from its last use, so one is asked for each time the frame opens.
  // Each time the frame opens, its key carries what the viewer's page stored since (ADR 0072).
  const [opening] = useState(() => Math.random().toString(36).slice(2))
  const address = useQuery({
    queryKey: ['artifact-frame', thread, artifact.id, artifact.version, startTheme, opening],
    queryFn: () =>
      api.artifactFrame(thread, artifact.id, {
        version: artifact.version,
        theme: startTheme,
        parent: window.location.origin,
        boot: { storage: keptFor(thread, artifact.id), hot: lastSnapshot(thread, artifact.id) },
      }),
    enabled: artifact.kind === 'html',
    staleTime: Infinity,
    gcTime: 0,
    refetchOnWindowFocus: false,
  })
  // Where no frame address can be had, the page is composed here as before.
  const [framed] = useState(() =>
    artifact.kind === 'html' ? sandboxedDocument(textOf(content), { style, theme: startTheme, vendor, parent: window.location.origin }) : '',
  )
  // The page's Mermaid diagrams, drawn here in the viewer's theme and sent back as images (ADR 0073).
  const diagrams = useRef<{ i: number; text: string }[]>([])
  const drawDiagrams = useRef((shownTheme: FrameTheme) => {
    const target = frame.current?.contentWindow
    for (const diagram of diagrams.current) {
      mermaidSvg(diagram.text, `frame-mermaid-${diagram.i}`, shownTheme === 'dark' ? 'dark' : 'neutral').then(
        (svg) => target?.postMessage({ silt: 'mermaid-drawn', i: diagram.i, src: svgUrl(svg) }, '*'),
        () => undefined,
      )
    }
  })
  useEffect(() => {
    frame.current?.contentWindow?.postMessage({ silt: 'theme', theme }, '*')
    drawDiagrams.current(theme)
  }, [theme])
  const shownTheme = useRef(theme)
  useEffect(() => {
    shownTheme.current = theme
  })
  // A frame's page says what's selected in it and what went wrong; only messages from that frame count. The listener
  // is added once, reading the latest callbacks, so a render doesn't drop a message between removing and adding it.
  const snapshots = useRef<number | null>(null)
  useEffect(
    () => () => {
      if (snapshots.current !== null) window.clearInterval(snapshots.current)
    },
    [],
  )
  const callbacks = useRef({ onSelect, onProblems })
  useEffect(() => {
    callbacks.current = { onSelect, onProblems }
  })
  useEffect(() => {
    // The bridge (ADR 0072): calls the page makes through `silt.use`, answered here.
    const broker = new Broker(
      {
        thread,
        artifact,
        emit: (cap, topic, data) => frame.current?.contentWindow?.postMessage({ silt: 'event', cap, topic, data }, '*'),
      },
      handlersFor(thread, artifact),
    )
    const heard = (event: MessageEvent) => {
      const said = event.data as { silt?: unknown; text?: unknown; problems?: unknown } | null
      if (event.source === null || event.source !== frame.current?.contentWindow) return
      if (said?.silt === 'selection') callbacks.current.onSelect(typeof said.text === 'string' ? said.text.slice(0, 500) : '')
      if (said?.silt === 'problems' && Array.isArray(said.problems)) callbacks.current.onProblems(said.problems.filter(isProblem).slice(0, 20))
      const call = callOf(said)
      if (call) {
        void broker.answer(call).then((answer) => frame.current?.contentWindow?.postMessage({ silt: 'reply', id: call.id, ...answer }, '*'))
      }
      if (said?.silt === 'storage') {
        const stored = said as { kind?: unknown; items?: unknown }
        keep(thread, artifact.id, stored.kind, stored.items)
      }
      if (said?.silt === 'hot' && (said as { keeps?: unknown }).keeps === true && snapshots.current === null) {
        snapshots.current = window.setInterval(() => frame.current?.contentWindow?.postMessage({ silt: 'snapshot' }, '*'), 2000)
      }
      if (said?.silt === 'snapshotted') rememberSnapshot(thread, artifact.id, (said as { data?: unknown }).data)
      if (said?.silt === 'mermaid' && Array.isArray((said as { diagrams?: unknown }).diagrams)) {
        diagrams.current = ((said as { diagrams: unknown[] }).diagrams)
          .filter((diagram): diagram is { i: number; text: string } => {
            const candidate = diagram as { i?: unknown; text?: unknown }
            return typeof candidate.i === 'number' && typeof candidate.text === 'string' && candidate.text.length <= 65536
          })
          .slice(0, 20)
        drawDiagrams.current(shownTheme.current)
      }
    }
    window.addEventListener('message', heard)
    return () => window.removeEventListener('message', heard)
  }, [thread, artifact])
  const selected = () => onSelect(String(window.getSelection() ?? '').trim().slice(0, 500))
  const scale = zoom / 100
  const zoomed = { transform: `scale(${scale})`, transformOrigin: '0 0', width: `${100 / scale}%`, height: `${100 / scale}%` }
  if (source)
    return (
      <pre onMouseUp={selected} className="h-full overflow-auto px-4 pt-4 pb-6 font-mono text-xs leading-5 whitespace-pre-wrap text-fg-1" data-testid="artifact-source">
        {textOf(content)}
      </pre>
    )
  if (artifact.kind === 'html')
    return (
      <div className="absolute inset-0 overflow-hidden bg-bg">
        <iframe
          ref={frame}
          title={artifact.title}
          sandbox="allow-scripts"
          allow="fullscreen; clipboard-write"
          {...(address.data ? { src: address.data.path } : address.isError ? { srcDoc: framed } : {})}
          referrerPolicy="no-referrer"
          style={zoomed}
          className="block border-0"
          data-testid="artifact-frame"
        />
      </div>
    )
  const page = (children: ReactNode) => (
    <div className="h-full overflow-auto" onMouseUp={selected}>
      <div style={{ zoom: scale }} className={cx('mx-auto px-6 pt-6 pb-8', full ? 'max-w-4xl' : '')}>
        {children}
      </div>
    </div>
  )
  if (artifact.kind === 'markdown') return page(<Markdown text={textOf(content)} />)
  if (artifact.kind === 'mermaid') return page(<Mermaid text={textOf(content)} title={artifact.title} />)
  return page(<img src={dataUrl(content)} alt={artifact.title} className="mx-auto block max-w-full rounded-sm bg-white" />)
}

export function Mermaid({ text, title }: { text: string; title: string }) {
  const id = `mermaid${useId()}`
  const [drawn, setDrawn] = useState<{ svg: string } | { error: string } | null>(null)
  useEffect(() => {
    let gone = false
    mermaidSvg(text, id).then(
      (svg) => !gone && setDrawn({ svg }),
      (failure: unknown) => !gone && setDrawn({ error: message(failure) }),
    )
    return () => {
      gone = true
    }
  }, [text, id])
  if (!drawn) return <Spinner />
  if ('error' in drawn)
    return (
      <div className="flex flex-col gap-2">
        <p className="text-sm text-danger">The diagram doesn't draw: {drawn.error}</p>
        <pre className="font-mono text-xs whitespace-pre-wrap text-fg-2">{text}</pre>
      </div>
    )
  return <img src={svgUrl(drawn.svg)} alt={title} className="mx-auto block max-w-full rounded-sm bg-white p-3" data-testid="artifact-diagram" />
}

/** What an offline copy shows for an artifact that isn't a page of its own: Markdown's markup, a diagram's SVG. */
async function renderedMarkup(artifact: ArtifactView, content: ArtifactContent): Promise<string | undefined> {
  if (artifact.kind === 'markdown') {
    const { renderToStaticMarkup } = await import('react-dom/server')
    return renderToStaticMarkup(<Markdown text={textOf(content)} />)
  }
  if (artifact.kind === 'mermaid') return mermaidSvg(textOf(content), `export-${artifact.blob.slice(0, 8)}`)
  if (artifact.kind === 'html') return withDiagramsDrawn(textOf(content))
  return undefined
}
