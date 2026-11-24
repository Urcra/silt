// What agents showed the person in the conversation (ADR 0068): images and videos,
// and cards that open an artifact beside it. Kept apart from the panel's
// viewer, which loads only once an artifact opens.

import { useContext, useState } from 'react'
import type { HistoryEntry } from '../api'
import { ArtifactActions } from '../artifactActions'
import { kindLabel } from '../artifacts'
import { artifactMedia, clock, formatOf, sizeOf, thumbSrc } from '../mediaInfo'
import { Icon, cx } from '../ui'
import { KeptHereOnly, MediaViewer, Unavailable, VideoPlayer } from './Media'

/**
 * An image or a video an agent showed in the conversation, with its caption (ADR 0120). An image shows its smallest
 * preview and opens in the viewer; a video plays where it is and opens in the viewer at the moment it's at.
 */
export function ShownImage({ thread, entry, tile = false }: { thread: string; entry: HistoryEntry; tile?: boolean }) {
  const artifact = entry.artifact!
  const media = artifactMedia(thread, artifact)
  const [open, setOpen] = useState<{ at?: number } | null>(null)
  const [missing, setMissing] = useState(false)
  const [seen, setSeen] = useState<{ duration: number; width: number; height: number } | null>(null)
  // A video's length and pixels, once the player has read them.
  const facts = seen
    ? `${formatOf(artifact.media_type)} · ${clock(seen.duration)} · ${seen.width}×${seen.height} · ${sizeOf(artifact.size)}`
    : media.meta
  const caption = (
    <>
      <figcaption className="text-2xs text-fg-3">
        {artifact.caption && <span className="text-fg-2">{artifact.caption} · </span>}
        {facts}
      </figcaption>
      <KeptHereOnly state={media.state} />
    </>
  )
  // In a grid of them, each is a tile of one size; a video shows its first frame and plays in the viewer.
  if (tile)
    return (
      <figure className="flex min-w-0 flex-col gap-1.5" data-testid={media.kind === 'video' ? 'shown-video' : 'shown-image'}>
        <button
          type="button"
          aria-label={`open the ${media.kind} ${artifact.caption ?? artifact.title}`}
          onClick={() => setOpen({})}
          className="relative aspect-video w-full cursor-zoom-in overflow-hidden rounded-md border border-line bg-surface-1 hover:border-line-strong"
        >
          {media.kind === 'video' ? (
            <>
              <video src={media.src} preload="metadata" muted playsInline className="h-full w-full object-cover" />
              <span className="absolute inset-0 m-auto grid h-9 w-9 place-items-center rounded-full border border-white/20 bg-[#140e08]/60 text-[#f4ecdd]">
                <Icon name="playSolid" className="ml-0.5 h-4 w-4" />
              </span>
            </>
          ) : (
            <img
              src={thumbSrc(thread, artifact.blob, artifact.previews)}
              alt={artifact.caption ?? artifact.title}
              className="h-full w-full object-contain"
            />
          )}
        </button>
        {caption}
        {open && <MediaViewer media={media} onClose={() => setOpen(null)} />}
      </figure>
    )
  if (media.kind === 'video')
    return (
      <figure className="flex w-fit max-w-full flex-col gap-1.5" data-testid="shown-video">
        <VideoPlayer media={media} variant="inline" onExpand={(at) => setOpen({ at })} onMetadata={setSeen} />
        {caption}
        {open && <MediaViewer media={media} startAt={open.at ?? 0} onClose={() => setOpen(null)} />}
      </figure>
    )
  const alt = artifact.caption ?? artifact.title
  return (
    <figure className="flex w-fit max-w-full flex-col gap-1.5" data-testid="shown-image">
      {missing && media.state ? (
        <div className="grid aspect-video w-[420px] max-w-full place-items-center rounded-md border border-line bg-surface-1 p-4">
          <Unavailable state={media.state} kind="image" onBack={() => setMissing(false)} />
        </div>
      ) : (
        <button
          type="button"
          aria-label={`open the image ${alt}`}
          onClick={() => setOpen({})}
          className="flex min-h-24 min-w-32 cursor-zoom-in items-center justify-center overflow-hidden rounded-md border border-line bg-surface-1 hover:border-line-strong"
        >
          <img
            src={thumbSrc(thread, artifact.blob, artifact.previews)}
            alt={alt}
            onError={() => setMissing(true)}
            className="block max-h-72 max-w-full object-contain md:max-w-[420px]"
          />
        </button>
      )}
      {caption}
      {open && <MediaViewer media={media} onClose={() => setOpen(null)} />}
    </figure>
  )
}

export function KindIcon({ kind, className }: { kind: string; className?: string }) {
  const name =
    kind === 'image' || kind === 'svg' || kind === 'canvas'
      ? 'image'
      : kind === 'video'
        ? 'video'
        : kind === 'mermaid'
          ? 'graph'
          : kind === 'slides'
            ? 'play'
            : kind === 'data'
              ? 'list'
              : 'file'
  return <Icon name={name} className={className} />
}

/** An artifact version as the conversation shows it where the agent made it: a card that opens it beside the conversation. */
export function ArtifactCard({ entry }: { entry: HistoryEntry }) {
  const artifact = entry.artifact!
  const actions = useContext(ArtifactActions)
  const open = actions?.opened?.id === artifact.id
  return (
    <button
      type="button"
      data-testid="artifact-card"
      onClick={() => actions?.open(artifact.id, artifact.version)}
      aria-label={`Open the artifact ${artifact.title}, version ${artifact.version}`}
      className={cx(
        'flex w-full max-w-[420px] items-center gap-3 rounded-md border bg-elevated px-3 py-2.5 text-left hover:border-line-strong',
        open ? 'border-accent shadow-[0_0_0_3px_var(--accent-tint)]' : 'border-line',
      )}
    >
      <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-sm bg-accent-tint text-accent-fg">
        <KindIcon kind={artifact.kind} className="h-5 w-5" />
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate font-medium">{artifact.title}</span>
        <span className="text-xs text-fg-2">
          {kindLabel(artifact.kind)} · v{artifact.version}
          {artifact.note ? ` · ${artifact.note}` : ''}
        </span>
        {artifact.description ? <span className="line-clamp-2 text-xs text-fg-3">{artifact.description}</span> : null}
      </span>
      <span className="shrink-0 text-xs text-accent-fg">{open ? 'Open beside' : 'Open'}</span>
    </button>
  )
}
