// The Artifacts page (ADR 0075): every artifact in the threads the person can read, one row each, pinned ones first,
// then the most recently changed; filters by project, kind, the person's own threads and text.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useMemo, useState } from 'react'
import { api, short, type ArtifactListed } from '../api'
import { artifactPath, kindLabel, listingMatches } from '../artifacts'
import { useTopic } from '../bus'
import { useNow } from '../now'
import { keys, message } from '../query'
import { ago } from '../time'
import { EmptyState, Icon, IconButton, Input, Skeleton, cx, toast } from '../ui'
import { KindIcon } from './ArtifactCards'
import { Page } from './Page'

const KINDS = ['html', 'svg', 'markdown', 'mermaid', 'image']

export function Gallery() {
  useTopic('threads')
  const now = useNow()
  const queries = useQueryClient()
  const listing = useQuery({ queryKey: keys.artifacts, queryFn: api.artifacts, refetchInterval: 15_000 })
  const [text, setText] = useState('')
  const [kind, setKind] = useState('')
  const [repo, setRepo] = useState('')
  const [mineOnly, setMineOnly] = useState(false)
  const [archived, setArchived] = useState(false)
  const rows = useMemo(() => listing.data ?? [], [listing.data])
  const repos = useMemo(() => [...new Set(rows.map((row) => row.repo))].sort(), [rows])
  const shown = rows.filter((row) => listingMatches(row, { text, kind, repo, mine: mineOnly, archived }))
  const pin = async (row: ArtifactListed) => {
    try {
      await api.pinArtifact(row.thread, row.artifact.id, !row.pinned)
      await queries.invalidateQueries({ queryKey: keys.artifacts })
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  const select = 'h-8 rounded-sm border border-line bg-surface-1 px-2 text-sm text-fg-1'
  return (
    <Page title="Artifacts" label="artifacts" width="wide">
      <div className="flex flex-col gap-4 px-4 py-4 md:px-6" data-testid="gallery">
        <div className="flex flex-wrap items-center gap-2">
          <Input
            aria-label="search artifacts"
            placeholder="Search titles and descriptions"
            value={text}
            onChange={(event) => setText(event.target.value)}
            className="min-w-0 flex-1 basis-56"
          />
          <select aria-label="kind" value={kind} onChange={(event) => setKind(event.target.value)} className={select}>
            <option value="">Every kind</option>
            {KINDS.map((candidate) => (
              <option key={candidate} value={candidate}>
                {kindLabel(candidate)}
              </option>
            ))}
          </select>
          {repos.length > 1 && (
            <select aria-label="project" value={repo} onChange={(event) => setRepo(event.target.value)} className={select}>
              <option value="">Every project</option>
              {repos.map((candidate) => (
                <option key={candidate} value={candidate}>
                  {candidate.split('/').pop()}
                </option>
              ))}
            </select>
          )}
          {rows.some((row) => !row.mine) && (
            <label className="inline-flex items-center gap-1.5 text-sm text-fg-2">
              <input type="checkbox" checked={mineOnly} onChange={(event) => setMineOnly(event.target.checked)} /> Made in my
              threads
            </label>
          )}
          <label className="inline-flex items-center gap-1.5 text-sm text-fg-2">
            <input type="checkbox" checked={archived} onChange={(event) => setArchived(event.target.checked)} /> Archived
          </label>
        </div>
        {listing.isPending ? (
          <div className="flex flex-col gap-2">
            <Skeleton />
            <Skeleton className="h-4 w-2/3" />
          </div>
        ) : listing.error ? (
          <p className="text-sm text-danger">{message(listing.error)}</p>
        ) : shown.length === 0 ? (
          <EmptyState title={rows.length === 0 ? 'No artifacts yet' : 'Nothing matches'}>
            {rows.length === 0
              ? 'Pages, charts and documents agents make for you in any thread show here.'
              : 'Try fewer filters.'}
          </EmptyState>
        ) : (
          <ul className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3" aria-label="artifacts">
            {shown.map((row) => (
              <li
                key={`${row.thread}/${row.artifact.id}`}
                className="relative flex flex-col rounded-md border border-line bg-elevated hover:border-line-strong"
                data-testid="gallery-card"
              >
                <Link to={artifactPath(row.thread, row.artifact.id)} className="flex min-w-0 flex-1 flex-col gap-2 p-3">
                  <Thumbnail row={row} />
                  <span className="truncate font-medium text-fg-1">{row.artifact.title}</span>
                  {row.artifact.description && <span className="line-clamp-2 text-xs text-fg-2">{row.artifact.description}</span>}
                  <span className="mt-auto flex min-w-0 items-center gap-1.5 font-mono text-2xs text-fg-3">
                    <span className="truncate">{row.thread_title ?? short(row.thread)}</span>
                    <span aria-hidden="true">·</span>
                    <span className="shrink-0">
                      {kindLabel(row.artifact.kind)} v{row.artifact.version}
                    </span>
                    <span aria-hidden="true">·</span>
                    <span className="shrink-0">{ago(row.time_ms, now)}</span>
                  </span>
                </Link>
                <IconButton
                  icon="check"
                  label={row.pinned ? `Unpin ${row.artifact.title}` : `Pin ${row.artifact.title}`}
                  aria-pressed={row.pinned}
                  onClick={() => void pin(row)}
                  className={cx('absolute top-2 right-2', row.pinned ? 'text-accent' : 'opacity-60 hover:opacity-100')}
                />
              </li>
            ))}
          </ul>
        )}
        <p className="flex items-center gap-1.5 text-xs text-fg-3">
          <Icon name="info" className="h-3.5 w-3.5" /> Only artifacts in threads you can read are listed.
        </p>
      </div>
    </Page>
  )
}

/** A version's thumbnail where one has been drawn, else its kind's icon; an image shows itself. */
function Thumbnail({ row }: { row: ArtifactListed }) {
  const drawn = useQuery({
    queryKey: ['artifact-thumbnail', row.thread, row.artifact.id, row.artifact.version],
    queryFn: () =>
      row.artifact.kind === 'image' || row.artifact.kind === 'svg'
        ? api.artifact(row.thread, row.artifact.blob)
        : api.artifactThumbnail(row.thread, row.artifact.id),
    enabled: ['html', 'image', 'svg'].includes(row.artifact.kind),
    staleTime: Infinity,
    retry: false,
  })
  const box = 'flex h-24 items-center justify-center overflow-hidden rounded-sm bg-surface-1 text-fg-2'
  if (drawn.data)
    return (
      <span className={box}>
        <img
          src={`data:${drawn.data.media_type};base64,${drawn.data.data}`}
          alt=""
          className="h-full w-full object-cover object-top"
          data-testid="gallery-thumbnail"
        />
      </span>
    )
  return (
    <span className={box}>
      <KindIcon kind={row.artifact.kind} className="h-7 w-7" />
    </span>
  )
}
