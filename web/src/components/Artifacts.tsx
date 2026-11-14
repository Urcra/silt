// What agents showed the person (ADR 0067): images in the conversation,
// artifact cards there, and the panel that shows an artifact beside it.

import { useQuery } from '@tanstack/react-query'
import { useEffect, useId, useRef, useState, type ReactNode } from 'react'
import { api, type ArtifactContent, type ArtifactView, type CommentView } from '../api'
import {
  type ArtifactGroup,
  type ArtifactVersion,
  dataUrl,
  fileName,
  kindLabel,
  latest,
  offlineHtml,
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
import { Dialog, Icon, IconButton, Menu, Spinner, cx, toast } from '../ui'
import { ArtifactCommentsButton, AskForChange } from './ArtifactComments'
import { KindIcon, ShownImage } from './ArtifactCards'
import { Markdown } from './Markdown'

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
const PILL = 'inline-flex h-8 items-center rounded-lg border border-line bg-elevated shadow-overlay max-md:h-11'

/**
 * An open artifact (ADR 0067): its header takes the panel's top row, its page fills the rest edge to edge, and its
 * version and view float over the page's top.
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
  const link = usePermalink(thread)
  const now = useNow()
  if (!group || !shown) return <p className="px-5 py-6 text-sm text-fg-2">This artifact isn't in the thread's history.</p>
  const artifact = shown.artifact
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
      download(offlineHtml(artifact, content.data, rendered), 'text/html', fileName(artifact, 'html'))
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  const latestVersion = latest(group).artifact.version
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="artifact-viewer">
      <div className="sticky top-0 z-20 flex h-12 shrink-0 items-center gap-0.5 border-b border-line-subtle bg-bg pr-2 pl-1.5">
        <button
          type="button"
          onClick={onBack}
          title="Changes, Artifacts, Details, Lineage"
          className="inline-flex items-center gap-1 rounded-sm px-1.5 py-1 text-sm text-fg-2 hover:bg-surface-2 hover:text-fg-1 max-md:min-h-11"
        >
          <Icon name="chevronLeft" className="h-3.5 w-3.5" />
          Artifacts
        </button>
        <span aria-hidden="true" className="text-fg-3">
          /
        </span>
        <span className="min-w-0">
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
        <span className="flex-1" />
        <Menu
          label="Share"
          trigger={
            <>
              <Icon name="lock" className="h-4 w-4" />
              <span className="max-md:sr-only">Share</span>
            </>
          }
          triggerClassName="inline-flex items-center gap-1.5 rounded-sm px-2 py-1 text-sm text-fg-1 hover:bg-surface-2 max-md:min-h-11"
          items={[
            { label: 'Read-only link…', icon: 'lock', hint: 'no account needed', onSelect: () => setSharing(true) },
            { label: 'Copy link', icon: 'link', hint: 'for people in the thread', onSelect: () => link(`/t/${thread}#artifact-${artifact.id}`) },
            { label: 'Download as one HTML file', icon: 'download', hint: 'opens anywhere', onSelect: () => void exportHtml(), divided: true },
            { label: 'Download the file', icon: 'file', onSelect: downloadFile },
          ]}
        />
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
        <div className="pointer-events-none absolute inset-x-0 top-0 z-10 flex items-center gap-2 p-2.5">
          <span className="pointer-events-auto">
            <Menu
              label="Versions"
              align="left"
              trigger={
                <>
                  v{artifact.version}
                  <Icon name="chevronDown" className="h-3 w-3 text-fg-2" />
                </>
              }
              triggerClassName={cx(PILL, 'gap-1.5 px-3 text-sm')}
              items={[...group.versions].reverse().map((version) => ({
                label: `v${version.artifact.version}${version.artifact.version === latestVersion ? ' · latest' : ''}`,
                hint: ago(version.entry.time_ms, now),
                icon: version.artifact.version === artifact.version ? 'check' : undefined,
                onSelect: () => onOpen(group.id, version.artifact.version === latestVersion ? null : version.artifact.version),
              }))}
            />
          </span>
          <span className="flex-1" />
          <span className={cx(PILL, 'pointer-events-auto gap-0.5 px-1')}>
            <IconButton icon="eye" label="Preview" aria-pressed={!source} onClick={() => setSource(false)} className="aria-pressed:bg-surface-2 aria-pressed:text-fg-1" />
            {textual && (
              <IconButton icon="code" label="Source" aria-pressed={source} onClick={() => setSource(true)} className="aria-pressed:bg-surface-2 aria-pressed:text-fg-1" />
            )}
            <span aria-hidden="true" className="mx-0.5 h-4 w-px bg-line" />
            <IconButton icon="reload" label="Reload" onClick={() => setReloads(reloads + 1)} />
            <IconButton icon="download" label="Download the file" onClick={downloadFile} />
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
          </span>
        </div>
        {content.data ? (
          <ArtifactPage
            key={`${artifact.blob}-${reloads}`}
            artifact={artifact}
            content={content.data}
            source={source}
            zoom={zoom}
            full={Boolean(full)}
            onSelect={(text) => setQuote(text || null)}
          />
        ) : content.error ? (
          <p className="px-5 pt-16 text-sm text-danger">{message(content.error)}</p>
        ) : (
          <div className="flex justify-center pt-20">
            <Spinner />
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

/** The page itself: an HTML artifact in a frame that can reach nothing, anything else drawn here without running it. */
function ArtifactPage({
  artifact,
  content,
  source,
  zoom,
  full,
  onSelect,
}: {
  artifact: ArtifactView
  content: ArtifactContent
  source: boolean
  zoom: number
  full: boolean
  onSelect: (text: string) => void
}) {
  const frame = useRef<HTMLIFrameElement>(null)
  // A frame's page says what's selected in it; only messages from that frame count.
  useEffect(() => {
    const heard = (event: MessageEvent) => {
      const said = event.data as { silt?: unknown; text?: unknown } | null
      if (event.source !== frame.current?.contentWindow || said?.silt !== 'selection') return
      onSelect(typeof said.text === 'string' ? said.text.slice(0, 500) : '')
    }
    window.addEventListener('message', heard)
    return () => window.removeEventListener('message', heard)
  }, [onSelect])
  const selected = () => onSelect(String(window.getSelection() ?? '').trim().slice(0, 500))
  const scale = zoom / 100
  const zoomed = { transform: `scale(${scale})`, transformOrigin: '0 0', width: `${100 / scale}%`, height: `${100 / scale}%` }
  if (source)
    return (
      <pre onMouseUp={selected} className="h-full overflow-auto px-4 pt-14 pb-6 font-mono text-xs leading-5 whitespace-pre-wrap text-fg-1" data-testid="artifact-source">
        {textOf(content)}
      </pre>
    )
  if (artifact.kind === 'html')
    return (
      <div className="absolute inset-0 overflow-hidden bg-white">
        <iframe
          ref={frame}
          title={artifact.title}
          sandbox="allow-scripts"
          srcDoc={sandboxedDocument(textOf(content))}
          referrerPolicy="no-referrer"
          style={zoomed}
          className="block border-0"
          data-testid="artifact-frame"
        />
      </div>
    )
  const page = (children: ReactNode) => (
    <div className="h-full overflow-auto" onMouseUp={selected}>
      <div style={{ zoom: scale }} className={cx('mx-auto px-6 pt-14 pb-8', full ? 'max-w-4xl' : '')}>
        {children}
      </div>
    </div>
  )
  if (artifact.kind === 'markdown') return page(<Markdown text={textOf(content)} />)
  if (artifact.kind === 'mermaid') return page(<Mermaid text={textOf(content)} title={artifact.title} />)
  return page(<img src={dataUrl(content)} alt={artifact.title} className="mx-auto block max-w-full rounded-sm bg-white" />)
}

/** Draws Mermaid as SVG, shown as an image so nothing in it runs. */
async function mermaidSvg(text: string, id: string): Promise<string> {
  const mermaid = (await import('mermaid')).default
  mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme: 'neutral', flowchart: { htmlLabels: false } })
  const { svg } = await mermaid.render(id.replace(/[^a-zA-Z0-9]/g, ''), text)
  return svg
}

function Mermaid({ text, title }: { text: string; title: string }) {
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
  return undefined
}
