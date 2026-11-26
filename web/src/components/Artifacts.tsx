// What agents showed the person (ADR 0068): images in the conversation,
// artifact cards there, and the panel that shows an artifact beside it.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { lazy, Suspense, useEffect, useId, useRef, useState, type ReactNode } from 'react'
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
  artifactPath,
  offlineHtml,
  type InlineFiles,
  problemsPrompt,
  sandboxedDocument,
  svgUrl,
  textOf,
  versionOf,
  commentsOn,
  quoted,
} from '../artifacts'
import type { Opened } from '../artifactActions'
import { canvasOf, canvasOffline, deckOf, deckOffline } from '../typed'
import { keys, message } from '../query'
import { hosted } from '../served'
import { useNow } from '../now'
import { usePermalink } from '../usePermalink'
import { Links } from './ShareDialog'
import { ago } from '../time'
import { useShownTheme } from '../theme'
import { Button, Dialog, Icon, IconButton, Menu, type MenuItem, Spinner, cx, toast } from '../ui'
import { ArtifactCommentsButton, AskForChange } from './ArtifactComments'
import { KindIcon, ShownImage } from './ArtifactCards'
import { artifactMedia } from '../mediaInfo'
import { VideoPlayer, ZoomableImage } from './Media'
import { Markdown } from './Markdown'
import { mermaidSvg, withDiagramsDrawn } from '../mermaid'
import { Broker, callOf } from '../bridge/broker'
import { handlersFor, type Holder } from '../bridge/capabilities'
import { lastSnapshot, rememberSnapshot } from '../bridge/hot'
import { keep, keptFor } from '../bridge/storage'

/** The Artifacts tab with none open: the thread's artifacts, the one changed last first, and the images agents showed. */
// The types' viewers load when one is opened, so a thread's page doesn't carry them (ADR 0076).
const SlidesView = lazy(() => import('./TypedArtifact').then((module) => ({ default: module.SlidesView })))
const CanvasView = lazy(() => import('./TypedArtifact').then((module) => ({ default: module.CanvasView })))
const DataView = lazy(() => import('./TypedArtifact').then((module) => ({ default: module.DataView })))
const DocEditor = lazy(() => import('./Edited').then((module) => ({ default: module.DocEditor })))
const SheetGrid = lazy(() => import('./Edited').then((module) => ({ default: module.SheetGrid })))
const WhiteboardView = lazy(() => import('./Whiteboard').then((module) => ({ default: module.WhiteboardView })))

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
  // Artifacts the thread's owner archived stay in the history but leave this list (ADR 0075).
  const listing = useQuery({ queryKey: keys.artifacts, queryFn: api.artifacts, staleTime: 15_000 })
  const archived = new Set(
    (listing.data ?? []).filter((row) => row.thread === thread && row.archived).map((row) => row.artifact.id),
  )
  groups = groups.filter((group) => !archived.has(group.id))
  if (groups.length === 0 && images.length === 0)
    return (
      <p className="px-5 py-6 text-sm text-fg-2">
        Nothing yet. When an agent you run here makes a page, a chart or a document for you, it shows here, and images it shows
        you are kept here too.
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
                    {kindLabel(last.artifact.kind)} ·{' '}
                    {group.versions.length === 1 ? '1 version' : `${group.versions.length} versions`}
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
          <h3 className="font-mono text-2xs tracking-[0.08em] text-fg-2 uppercase">Images and videos the agent showed</h3>
          <div className="grid grid-cols-2 gap-3 md:grid-cols-3">
            {images.map((image) => (
              <ShownImage key={image.entry.index} thread={thread} entry={image.entry} tile />
            ))}
          </div>
        </section>
      )}
    </div>
  )
}

const ZOOMS = [50, 75, 100, 125, 150, 200]
const PRESSED = 'aria-pressed:bg-surface-2 aria-pressed:text-fg-1'
const ICON_TRIGGER =
  'inline-flex items-center justify-center rounded-sm p-1 text-fg-2 hover:bg-surface-2 hover:text-fg-1 max-md:min-h-11 max-md:min-w-11'

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
  // The page's element a new comment is on, when the page opened the composer on one (ADR 0085).
  const [anchor, setAnchor] = useState<string | null>(null)
  const revealing = useRef<((anchor: string) => void) | null>(null)
  const [sharing, setSharing] = useState(false)
  // What a page asks the person to agree to, such as saving a file, until they answer.
  const [asking, setAsking] = useState<{
    title: string
    detail: string
    action: string
    answer: (agreed: boolean) => void
  } | null>(null)
  // Whether the page listens to the person or speaks to them now, and what stops it (ADR 0079).
  const [voice, setVoice] = useState<{ now: 'listening' | 'speaking'; stop: () => void } | null>(null)
  const composer = useRef<(() => void) | null>(null)
  const shown = group ? versionOf(group, opened.version) : null
  // What the holder's calls act on, as the view now stands; the holder itself is made once.
  const current = useRef({ group, shown: shown, comments, onCommentsChanged })
  useEffect(() => {
    current.current = { group, shown: shown, comments, onCommentsChanged }
  })
  // Asks wait their turn rather than replace the one shown, so a page can't swap what the person is about to agree to.
  // The agreeing button waits half a second after an ask appears, so a click meant for something else isn't its answer.
  const [armed, setArmed] = useState(false)
  const arm = useRef(() => {
    setArmed(false)
    window.setTimeout(() => setArmed(true), 500)
  })
  const waiting = useRef<{ title: string; detail: string; action: string; answer: (agreed: boolean) => void }[]>([])
  const [holder] = useState<Holder>(() => ({
    confirm: (ask) =>
      new Promise<boolean>((answer) => {
        if (waiting.current.length >= 3) return answer(false)
        waiting.current.push({ ...ask, answer })
        if (waiting.current.length === 1) {
          arm.current()
          setAsking(waiting.current[0])
        }
      }),
    openComposer: (text, anchor) => {
      setQuote(text || null)
      setAnchor(anchor ?? null)
      composer.current?.()
    },
    addComment: async (text, quote, anchor) => {
      const { shown, onCommentsChanged } = current.current
      if (!shown) throw new Error('no version is shown')
      const on = anchor
        ? ({ on: 'element', artifact: shown.artifact.id, anchor } as const)
        : ({ on: 'entry', index: shown.entry.index } as const)
      const made = await api.addComment(thread, on, quoted(text, quote))
      await onCommentsChanged()
      return made.id
    },
    commentIds: () => {
      const { group, comments } = current.current
      return group ? commentsOn(group, comments).map((comment) => comment.id) : []
    },
    commentsChanged: () => void current.current.onCommentsChanged(),
    revealer: (reveal) => {
      revealing.current = reveal
    },
    voice: (now, stop) => setVoice(now === 'idle' || !stop ? null : { now, stop }),
  }))
  // A click within half a second of an ask appearing isn't taken as its answer.
  const answerAsking = (agreed: boolean) => {
    if (!asking || (agreed && !armed)) return
    waiting.current.shift()
    asking.answer(agreed)
    arm.current()
    setAsking(waiting.current[0] ?? null)
  }
  const content = useQuery({
    queryKey: keys.artifact(thread, shown?.artifact.blob ?? ''),
    queryFn: () => api.artifact(thread, shown!.artifact.blob),
    // An image or a video is read by range from its blob rather than whole (ADR 0121).
    enabled: Boolean(shown) && !readByBlob(shown!.artifact),
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
  // Libraries and fonts come through Silt's vendor route when that's on, and always offline (ADR 0071, ADR 0134).
  const settings = useQuery({ queryKey: keys.artifactSettings, queryFn: api.artifactSettings, staleTime: 60_000 })
  const network = useQuery({
    queryKey: keys.networkSettings,
    queryFn: api.networkSettings,
    staleTime: 60_000,
    enabled: !hosted(),
  })
  const keptOnly = !hosted() && (network.data?.offline ?? network.isError)
  const vendor = settings.data?.proxy || keptOnly ? `${window.location.origin}/vendor/` : null
  // What this viewer's frame saw go wrong in the version shown, until it's sent or dismissed.
  const [problems, setProblems] = useState<{ blob: string; seen: FrameProblem[] } | null>(null)
  const link = usePermalink(thread)
  const now = useNow()
  const queries = useQueryClient()
  const listing = useQuery({ queryKey: keys.artifacts, queryFn: api.artifacts, staleTime: 15_000 })
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
  const latestVersion = latest(group).artifact.version
  const listed = listing.data?.find((row) => row.thread === thread && row.artifact.id === artifact.id)
  const keep = async (what: 'pin' | 'archive') => {
    try {
      if (what === 'pin') await api.pinArtifact(thread, artifact.id, !listed?.pinned)
      else await api.archiveArtifact(thread, artifact.id, !listed?.archived)
      await queries.invalidateQueries({ queryKey: keys.artifacts })
      toast(what === 'pin' ? (listed?.pinned ? 'Unpinned' : 'Pinned') : listed?.archived ? 'Unarchived' : 'Archived', 'ok')
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
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
      // The version's files go inside the copy, so it opens anywhere (ADR 0074).
      const files = await inlineFilesOf(thread, artifact)
      // A deck's copy is a page of its own: its slides in one frame, with the viewer's controls (ADR 0076).
      const deck = artifact.kind === 'slides' ? deckOf(textOf(content.data)) : null
      if (deck) {
        download(
          deckOffline(deck, `${artifact.title} · v${artifact.version}`, styleText, files),
          'text/html',
          fileName(artifact, 'html'),
        )
        return
      }
      const canvas = artifact.kind === 'canvas' ? canvasOf(textOf(content.data)) : null
      if (canvas) {
        download(
          canvasOffline(canvas, `${artifact.title} · v${artifact.version}`, styleText, files),
          'text/html',
          fileName(artifact, 'html'),
        )
        return
      }
      const rendered = await renderedMarkup(artifact, content.data)
      // Past what one HTML file carries well, the copy is the page with its files beside it, zipped.
      const carried = Object.values(files).reduce((sum, file) => sum + (file.data.length * 3) / 4, 0)
      if (carried > ZIP_ABOVE) {
        const { zipOf } = await import('../zip')
        const decoded = (data: string) => Uint8Array.from(atob(data), (char) => char.charCodeAt(0))
        const page = new TextEncoder().encode(offlineHtml(artifact, content.data, rendered, styleText, {}))
        const zip = zipOf([
          { path: 'index.html', bytes: page },
          ...Object.entries(files).map(([path, file]) => ({ path, bytes: decoded(file.data) })),
        ])
        download(zip, 'application/zip', fileName(artifact, 'zip'))
        return
      }
      download(offlineHtml(artifact, content.data, rendered, styleText, files), 'text/html', fileName(artifact, 'html'))
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  const zoomTo = (step: number) => ZOOMS[ZOOMS.indexOf(zoom) + step]
  const offline: MenuItem[] = offlineCopy(artifact)
    ? [
        {
          label: 'Download as one HTML file',
          icon: 'download',
          hint: 'opens anywhere',
          onSelect: () => void exportHtml(),
          divided: true,
        },
      ]
    : []
  const shareItems: MenuItem[] = [
    { label: 'Read-only link…', icon: 'lock', hint: 'no account needed', onSelect: () => setSharing(true) },
    {
      label: 'Copy link',
      icon: 'link',
      hint: 'for people in the thread',
      onSelect: () => link(artifactPath(thread, artifact.id, artifact.version === latestVersion ? null : artifact.version)),
    },
    {
      label: listed?.pinned ? 'Unpin from Artifacts' : 'Pin to Artifacts',
      icon: 'check',
      onSelect: () => void keep('pin'),
      divided: true,
    },
    {
      label: listed?.archived ? 'Unarchive' : 'Archive',
      icon: 'folder',
      hint: "the thread's owner's",
      onSelect: () => void keep('archive'),
    },
    ...offline,
    { label: 'Download the file', icon: 'file', onSelect: downloadFile, divided: !offlineCopy(artifact) },
  ]
  const viewItems: MenuItem[] = [
    { label: 'Preview', icon: source ? 'eye' : 'check', onSelect: () => setSource(false) },
    ...(textual
      ? [{ label: 'Source', icon: source ? 'check' : 'code', onSelect: () => setSource(true) } satisfies MenuItem]
      : []),
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
  const more = (items: MenuItem[]) => (
    <Menu label="More" trigger={<Icon name="more" className="h-4 w-4" />} triggerClassName={ICON_TRIGGER} items={items} />
  )
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
              hint: [
                version.artifact.by ? `saved by the page for ${version.artifact.by}` : null,
                version.artifact.note,
                ago(version.entry.time_ms, now),
              ]
                .filter(Boolean)
                .join(' · '),
              icon: version.artifact.version === artifact.version ? 'check' : undefined,
              onSelect: () => onOpen(group.id, version.artifact.version === latestVersion ? null : version.artifact.version),
            }))}
          />
        </span>
        <span className="min-w-2 flex-1" />
        {voice && (
          <span
            role="status"
            className="mr-1 inline-flex shrink-0 items-center gap-1.5 rounded-full border border-accent/40 bg-accent-tint py-0.5 pr-0.5 pl-2 text-xs text-fg-1"
            data-testid="page-voice"
          >
            <span aria-hidden="true" className="h-1.5 w-1.5 animate-pulse rounded-full bg-accent" />
            {voice.now === 'listening' ? 'The page is listening' : 'The page is speaking'}
            <button
              type="button"
              onClick={voice.stop}
              className="rounded-full px-2 py-0.5 font-medium text-accent-fg hover:bg-surface-2 max-md:min-h-11"
            >
              Stop
            </button>
          </span>
        )}
        <span
          role="group"
          aria-label="View"
          className="mr-1 inline-flex shrink-0 gap-px rounded-sm bg-surface-1 p-0.5 @max-md/artifact:hidden"
        >
          <IconButton icon="eye" label="Preview" aria-pressed={!source} onClick={() => setSource(false)} className={PRESSED} />
          {textual && (
            <IconButton icon="code" label="Source" aria-pressed={source} onClick={() => setSource(true)} className={PRESSED} />
          )}
        </span>
        <span
          className={cx(
            'inline-flex shrink-0 items-center gap-0.5 @max-xl/artifact:hidden',
            // A picture has its own zoom (ADR 0121).
            PICTURES.has(artifact.kind) && 'hidden',
          )}
        >
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
            items={ZOOMS.map((level) => ({
              label: `${level}%`,
              icon: level === zoom ? 'check' : undefined,
              onSelect: () => setZoom(level),
            }))}
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
        <span className="hidden shrink-0 @max-md/artifact:inline-flex">{more(narrowItems)}</span>
        <ArtifactCommentsButton
          thread={thread}
          group={group}
          shown={shown}
          comments={comments}
          quote={quote}
          anchor={anchor}
          onClearQuote={() => {
            setQuote(null)
            setAnchor(null)
          }}
          onChanged={onCommentsChanged}
          opener={(open) => {
            composer.current = open
          }}
          onReveal={(shownAnchor) => revealing.current?.(shownAnchor)}
        />
        {onFull && (
          <IconButton
            icon={full ? 'collapse' : 'expand'}
            label={full ? 'Leave the full view (F)' : 'Full view (F)'}
            onClick={onFull}
            className="max-lg:hidden"
          />
        )}
        <IconButton icon="x" label="Close the artifact" onClick={onClose} />
      </div>
      <div className="relative min-h-0 flex-1 overflow-hidden bg-elevated">
        {readByBlob(artifact) ? (
          <PanelMedia key={artifact.blob} thread={thread} artifact={artifact} />
        ) : content.data && styled ? (
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
            holder={holder}
            versions={group?.versions.map((version) => version.artifact) ?? []}
          />
        ) : content.error ? (
          <p className="px-5 py-6 text-sm text-danger">{message(content.error)}</p>
        ) : (
          <div className="flex justify-center pt-20">
            <Spinner />
          </div>
        )}
        {asking && (
          <div
            role="alertdialog"
            aria-label={asking.title}
            className="absolute inset-x-2.5 top-2.5 z-20 flex flex-wrap items-center gap-2 rounded-md border border-line bg-elevated px-3 py-2 text-xs text-fg-1 shadow-overlay"
            data-testid="artifact-asking"
          >
            <Icon name="lock" className="h-3.5 w-3.5 shrink-0 text-fg-2" />
            <span className="min-w-0 flex-1">
              <span className="font-medium">{asking.title}</span> <span className="break-all text-fg-2">{asking.detail}</span>
            </span>
            <Button size="sm" variant="ghost" onClick={() => answerAsking(false)}>
              Not now
            </Button>
            <Button size="sm" disabled={!armed} onClick={() => answerAsking(true)}>
              {asking.action}
            </Button>
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
              {seen.length === 1
                ? 'Your browser saw a problem in this version: '
                : `Your browser saw ${seen.length} problems in this version, first: `}
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
  holder,
  versions,
}: {
  thread: string
  artifact: ArtifactView
  /** Every version of the artifact, oldest first, for a doc to compare with. */
  versions: ArtifactView[]
  content: ArtifactContent
  style: string | null
  vendor: string | null
  source: boolean
  zoom: number
  full: boolean
  onSelect: (text: string) => void
  onProblems: (problems: FrameProblem[]) => void
  holder: Holder
}) {
  const frame = useRef<HTMLIFrameElement>(null)
  const theme = useShownTheme()
  // The frame starts in the theme shown when it loads, and hears of each change after, so its page isn't reloaded.
  const [startTheme] = useState(theme)
  // An HTML version loads from a frame address the daemon or server composes it at (ADR 0074). The key behind it
  // lasts an hour from its last use, so one is asked for each time the frame opens.
  // Each time the frame opens, its key carries what the viewer's page stored since (ADR 0072).
  const [opening] = useState(() => Math.random().toString(36).slice(2))
  // Every message the frame's page sends carries this; a document the frame navigates to doesn't have it, so it's
  // heard as nothing, and nothing is sent to it (ADR 0072).
  const [nonce] = useState(() => crypto.randomUUID().replace(/-/g, ''))
  const alive = useRef(true)
  const held = useRef<unknown[]>([])
  const send = useRef((message: unknown) => {
    if (alive.current) frame.current?.contentWindow?.postMessage(message, '*')
    else if (held.current.length < 200) held.current.push(message)
  })
  const loaded = () => {
    alive.current = false
    frame.current?.contentWindow?.postMessage({ silt: 'hello?' }, '*')
  }
  const address = useQuery({
    queryKey: ['artifact-frame', thread, artifact.id, artifact.version, startTheme, opening],
    queryFn: () =>
      api.artifactFrame(thread, artifact.id, {
        version: artifact.version,
        theme: startTheme,
        parent: window.location.origin,
        boot: {
          storage: keptFor(thread, artifact.id),
          hot: lastSnapshot(thread, artifact.id),
          contract: artifact.contract ?? '1',
          nonce,
        },
      }),
    enabled: artifact.kind === 'html',
    staleTime: Infinity,
    gcTime: 0,
    refetchOnWindowFocus: false,
  })
  // A plain `#name` in the artifact's link reaches the page, for its own views; `#artifact-…` is Silt's.
  const [pageHash] = useState(() =>
    /^#[A-Za-z0-9-]{1,64}$/.test(window.location.hash) && !window.location.hash.startsWith('#artifact-')
      ? window.location.hash
      : '',
  )
  // Where no frame address can be had, the page is composed here as before.
  const [framed] = useState(() =>
    artifact.kind === 'html'
      ? sandboxedDocument(textOf(content), {
          style,
          theme: startTheme,
          vendor,
          parent: window.location.origin,
          boot: JSON.stringify({ nonce }),
        })
      : '',
  )
  // The page's Mermaid diagrams, drawn here in the viewer's theme and sent back as images (ADR 0073).
  const diagrams = useRef<{ i: number; text: string }[]>([])
  const drawDiagrams = useRef((shownTheme: FrameTheme) => {
    for (const diagram of diagrams.current) {
      mermaidSvg(diagram.text, `frame-mermaid-${diagram.i}`, shownTheme === 'dark' ? 'dark' : 'neutral').then(
        (svg) => send.current({ silt: 'mermaid-drawn', i: diagram.i, src: svgUrl(svg) }),
        () => undefined,
      )
    }
  })
  useEffect(() => {
    send.current({ silt: 'theme', theme })
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
        emit: (cap, topic, data) => send.current({ silt: 'event', cap, topic, data }),
      },
      handlersFor(thread, artifact, holder),
    )
    const heard = (event: MessageEvent) => {
      const said = event.data as { silt?: unknown; text?: unknown; problems?: unknown; nonce?: unknown } | null
      if (event.source === null || event.source !== frame.current?.contentWindow || said?.nonce !== nonce) return
      if (!alive.current) {
        alive.current = true
        for (const message of held.current.splice(0)) send.current(message)
      }
      if (said?.silt === 'selection') callbacks.current.onSelect(typeof said.text === 'string' ? said.text.slice(0, 500) : '')
      if (said?.silt === 'problems' && Array.isArray(said.problems))
        callbacks.current.onProblems(said.problems.filter(isProblem).slice(0, 20))
      const call = callOf(said)
      if (call) {
        void broker.answer(call).then((answer) => send.current({ silt: 'reply', id: call.id, ...answer }))
      }
      if (said?.silt === 'storage') {
        const stored = said as { kind?: unknown; items?: unknown }
        keep(thread, artifact.id, stored.kind, stored.items)
      }
      if (said?.silt === 'hot' && (said as { keeps?: unknown }).keeps === true && snapshots.current === null) {
        // Not while the window is hidden, when nothing on the page changes.
        snapshots.current = window.setInterval(
          () => document.visibilityState === 'visible' && send.current({ silt: 'snapshot' }),
          2000,
        )
      }
      if (said?.silt === 'snapshotted') rememberSnapshot(thread, artifact.id, (said as { data?: unknown }).data)
      if (said?.silt === 'mermaid' && Array.isArray((said as { diagrams?: unknown }).diagrams)) {
        diagrams.current = (said as { diagrams: unknown[] }).diagrams
          .filter((diagram): diagram is { i: number; text: string } => {
            const candidate = diagram as { i?: unknown; text?: unknown }
            return typeof candidate.i === 'number' && typeof candidate.text === 'string' && candidate.text.length <= 65536
          })
          .slice(0, 20)
        drawDiagrams.current(shownTheme.current)
      }
    }
    window.addEventListener('message', heard)
    holder.revealer?.((anchor) => send.current({ silt: 'event', cap: 'comments', topic: 'reveal', data: { anchor } }))
    return () => {
      window.removeEventListener('message', heard)
      holder.revealer?.(null)
      broker.dispose()
    }
  }, [thread, artifact, holder, nonce])
  const selected = () =>
    onSelect(
      String(window.getSelection() ?? '')
        .trim()
        .slice(0, 500),
    )
  // A deck's or a canvas's own files, carried into their frames, which can't reach Silt (ADR 0074).
  const files = useQuery({
    queryKey: ['artifact-files', thread, artifact.id, artifact.version],
    queryFn: () => inlineFilesOf(thread, artifact),
    enabled: (artifact.kind === 'slides' || artifact.kind === 'canvas') && Object.keys(artifact.files ?? {}).length > 0,
    staleTime: Infinity,
  })
  const scale = zoom / 100
  const zoomed = { transform: `scale(${scale})`, transformOrigin: '0 0', width: `${100 / scale}%`, height: `${100 / scale}%` }
  if (source)
    return (
      <pre
        onMouseUp={selected}
        className="h-full overflow-auto px-4 pt-4 pb-6 font-mono text-xs leading-5 whitespace-pre-wrap text-fg-1"
        data-testid="artifact-source"
      >
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
          {...(address.data ? { src: address.data.path + pageHash } : address.isError ? { srcDoc: framed } : {})}
          referrerPolicy="no-referrer"
          onLoad={loaded}
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
  if (artifact.kind === 'mermaid') return <Mermaid text={textOf(content)} title={artifact.title} zoomable />
  const typed = (viewer: ReactNode) => <Suspense fallback={null}>{viewer}</Suspense>
  // Types Silt draws from the version's JSON (ADR 0076).
  if (artifact.kind === 'slides')
    return typed(
      <SlidesView
        text={textOf(content)}
        style={style}
        theme={startTheme}
        title={artifact.title}
        files={files.data}
        onComment={(quote) => holder.openComposer(quote)}
        onPrint={(html) => void printDeck(thread, artifact, html)}
      />,
    )
  if (artifact.kind === 'canvas')
    return typed(
      <CanvasView
        text={textOf(content)}
        style={style}
        theme={startTheme}
        files={files.data}
        onComment={(quote) => holder.openComposer(quote)}
      />,
    )
  if (artifact.kind === 'data')
    return typed(
      <DataView
        text={textOf(content)}
        theme={theme}
        style={style}
        live={{ thread, artifact: artifact.id, version: artifact.version }}
      />,
    )
  // Docs and sheets show as they stand now, edited in place (ADR 0077).
  if (artifact.kind === 'doc')
    return typed(<DocEditor thread={thread} artifact={artifact.id} title={artifact.title} versions={versions} />)
  if (artifact.kind === 'sheet') return typed(<SheetGrid thread={thread} artifact={artifact.id} title={artifact.title} />)
  if (artifact.kind === 'whiteboard')
    return typed(<WhiteboardView thread={thread} artifact={artifact.id} title={artifact.title} />)
  // A kind this UI doesn't know shows as its source rather than failing as an image.
  if (!artifact.media_type.startsWith('image/'))
    return (
      <pre
        className="h-full overflow-auto px-4 pt-4 pb-6 font-mono text-xs leading-5 whitespace-pre-wrap text-fg-1"
        data-testid="artifact-source"
      >
        {textOf(content)}
      </pre>
    )
  return <ZoomableImage layout="floating" media={{ kind: 'image', title: artifact.title, src: dataUrl(content) }} />
}

/** The kinds that are a picture, which zoom and pan rather than take the page's zoom. */
const PICTURES = new Set(['image', 'video', 'svg', 'mermaid'])

/** An image or a video, which the panel reads by range from its blob (ADR 0121); an SVG is read as text. */
function readByBlob(artifact: ArtifactView): boolean {
  return artifact.kind === 'video' || (artifact.kind === 'image' && artifact.media_type !== 'image/svg+xml')
}

/** An artifact's image, zoomable, or its video, beside the conversation. */
function PanelMedia({ thread, artifact }: { thread: string; artifact: ArtifactView }) {
  const media = artifactMedia(thread, artifact)
  if (media.kind === 'video')
    return (
      <div className="absolute inset-0 grid place-items-center overflow-auto bg-bg p-4">
        <VideoPlayer media={media} variant="panel" />
      </div>
    )
  return (
    <div className="absolute inset-0" data-testid="artifact-image">
      <ZoomableImage layout="floating" media={media} />
    </div>
  )
}

export function Mermaid({ text, title, zoomable = false }: { text: string; title: string; zoomable?: boolean }) {
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
  if (zoomable)
    return (
      <div className="absolute inset-0" data-testid="artifact-diagram">
        <ZoomableImage layout="floating" media={{ kind: 'image', title, src: svgUrl(drawn.svg) }} />
      </div>
    )
  return (
    <img
      src={svgUrl(drawn.svg)}
      alt={title}
      className="mx-auto block max-w-full rounded-sm bg-white p-3"
      data-testid="artifact-diagram"
    />
  )
}

/** What an offline copy shows for an artifact that isn't a page of its own: Markdown's markup, a diagram's SVG. */
/** A deck printed to a PDF by this computer's headless browser, handed to the person to save. */
async function printDeck(thread: string, artifact: ArtifactView, html: string) {
  try {
    const printed = await api.artifactPdf(thread, artifact.id, html)
    const url = URL.createObjectURL(
      new Blob([Uint8Array.from(atob(printed.data), (char) => char.charCodeAt(0))], { type: 'application/pdf' }),
    )
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = `${artifact.title.replace(/[^A-Za-z0-9_-]+/g, '-') || 'deck'}.pdf`
    anchor.click()
    setTimeout(() => URL.revokeObjectURL(url), 1000)
  } catch (failure) {
    toast(message(failure), 'danger')
  }
}

/** A version's files, read whole, as a frame or an offline copy carries them. */
async function inlineFilesOf(thread: string, artifact: ArtifactView): Promise<InlineFiles> {
  const files: InlineFiles = {}
  for (const [path, file] of Object.entries(artifact.files ?? {})) {
    if (!file) continue
    const read = await api.artifact(thread, file.blob)
    files[path] = { media_type: file.media_type, data: read.data }
  }
  return files
}

/** Whether a version has an offline copy: the kinds drawn from JSON that haven't one would show nothing. */
function offlineCopy(artifact: ArtifactView): boolean {
  return !['data', 'sheet', 'whiteboard'].includes(artifact.kind)
}

/** Past this many bytes of files, an offline copy is a zip rather than one HTML file. */
const ZIP_ABOVE = 32 << 20

async function renderedMarkup(artifact: ArtifactView, content: ArtifactContent): Promise<string | undefined> {
  if (artifact.kind === 'markdown' || artifact.kind === 'doc') {
    const { renderToStaticMarkup } = await import('react-dom/server')
    return renderToStaticMarkup(<Markdown text={textOf(content)} />)
  }
  if (artifact.kind === 'mermaid') return mermaidSvg(textOf(content), `export-${artifact.blob.slice(0, 8)}`)
  if (artifact.kind === 'html') return withDiagramsDrawn(textOf(content))
  return undefined
}
