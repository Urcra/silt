// What agents showed the person in the conversation (ADR 0067): images,
// and cards that open an artifact beside it. Kept apart from the panel's
// viewer, which loads only once an artifact opens.

import { useContext, useState } from 'react'
import type { HistoryEntry } from '../api'
import { ArtifactActions, useArtifactContent } from '../artifactActions'
import { dataUrl, kindLabel } from '../artifacts'
import { message } from '../query'
import { Dialog, Icon, Spinner, cx } from '../ui'

/** An image an agent showed in the conversation, with its caption; it opens at full size. */
export function ShownImage({ thread, entry }: { thread: string; entry: HistoryEntry }) {
  const artifact = entry.artifact!
  const content = useArtifactContent(thread, artifact)
  const [open, setOpen] = useState(false)
  const alt = artifact.caption ?? artifact.title
  return (
    <figure className="flex w-fit max-w-full flex-col gap-1.5" data-testid="shown-image">
      <button
        type="button"
        aria-label={`open the image ${alt}`}
        onClick={() => setOpen(true)}
        disabled={!content.data}
        className="flex min-h-24 min-w-32 items-center justify-center overflow-hidden rounded-md border border-line bg-surface-1 hover:border-line-strong"
      >
        {content.data ? (
          <img src={dataUrl(content.data)} alt={alt} className="block max-h-72 max-w-full object-contain md:max-w-[420px]" />
        ) : content.error ? (
          <span className="px-3 text-2xs text-fg-2">{message(content.error)}</span>
        ) : (
          <Spinner />
        )}
      </button>
      <figcaption className="text-2xs text-fg-3">
        {artifact.caption && <span className="text-fg-2">{artifact.caption} · </span>}
        {artifact.media_type.replace(/^image\//, '').toUpperCase()} · {sizeOf(artifact.size)}
      </figcaption>
      {content.data && (
        <Dialog open={open} onClose={() => setOpen(false)} title={alt} wide>
          <img src={dataUrl(content.data)} alt={alt} className="max-h-[75vh] w-full object-contain" />
        </Dialog>
      )}
    </figure>
  )
}

function sizeOf(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

export function KindIcon({ kind, className }: { kind: string; className?: string }) {
  return <Icon name={kind === 'image' || kind === 'svg' ? 'image' : kind === 'mermaid' ? 'graph' : 'file'} className={className} />
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
        </span>
      </span>
      <span className="shrink-0 text-xs text-accent-fg">{open ? 'Open beside' : 'Open'}</span>
    </button>
  )
}
