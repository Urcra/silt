// A read-only link to one artifact (ADR 0027, ADR 0068): the server serves the app's bundle at `/r/<token>`, and this
// page shows the artifact as the web UI does, with nothing else of the thread and no account.

import { useEffect, useState } from 'react'
import type { ArtifactView } from '../api'
import { Markdown } from './Markdown'
import { Mermaid } from './Artifacts'

type Linked = { artifact: ArtifactView; owner: string; frozen_ms: number | null; expires_ms: number }

type Loaded = { state: 'loading' } | { state: 'gone' } | { state: 'shown'; linked: Linked; text: string | null }

export function LinkedArtifact({ token }: { token: string }) {
  const [loaded, setLoaded] = useState<Loaded>({ state: 'loading' })
  useEffect(() => {
    let live = true
    void (async () => {
      const view = await fetch(`/r/${token}/view`, { credentials: 'omit' })
      if (!view.ok) {
        if (live) setLoaded({ state: 'gone' })
        return
      }
      const linked = (await view.json()) as Linked
      const textual = linked.artifact.kind === 'markdown' || linked.artifact.kind === 'mermaid'
      const text = textual ? await (await fetch(`/r/${token}/content`, { credentials: 'omit' })).text() : null
      if (live) setLoaded({ state: 'shown', linked, text })
    })().catch(() => live && setLoaded({ state: 'gone' }))
    return () => {
      live = false
    }
  }, [token])

  if (loaded.state === 'loading') return <main className="mx-auto max-w-4xl px-4 py-8 text-fg-3">Opening…</main>
  if (loaded.state === 'gone')
    return (
      <main className="mx-auto max-w-2xl px-4 py-12 text-fg-1">
        <h1 className="text-title font-semibold">Not available</h1>
        <p className="mt-2 text-fg-2">This link expired, was revoked, or never existed.</p>
      </main>
    )
  const { linked, text } = loaded
  const { artifact } = linked
  const content = `/r/${token}/content`
  return (
    <main className="flex min-h-screen flex-col bg-bg text-fg-1">
      <header className="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-border px-4 py-3">
        <h1 className="text-title-sm font-semibold">{artifact.title}</h1>
        <span className="text-small text-fg-3">
          v{artifact.version} · shared by {linked.owner}
          {linked.frozen_ms !== null ? ` · as of ${new Date(linked.frozen_ms).toLocaleString()}` : ''}
        </span>
        {artifact.description ? <p className="w-full text-small text-fg-2">{artifact.description}</p> : null}
      </header>
      <div className="relative min-h-0 flex-1">
        {artifact.kind === 'html' ? (
          <iframe
            src={content}
            title={artifact.title}
            sandbox="allow-scripts"
            referrerPolicy="no-referrer"
            className="absolute inset-0 block h-full w-full border-0"
            data-testid="linked-frame"
          />
        ) : artifact.kind === 'markdown' ? (
          <div className="mx-auto max-w-4xl px-6 pt-6 pb-8">
            <Markdown text={text ?? ''} />
          </div>
        ) : artifact.kind === 'mermaid' ? (
          <div className="mx-auto max-w-4xl px-6 pt-6 pb-8">
            <Mermaid text={text ?? ''} title={artifact.title} />
          </div>
        ) : (
          <div className="mx-auto max-w-4xl px-6 pt-6 pb-8">
            <img src={content} alt={artifact.title} className="mx-auto block max-w-full rounded-sm bg-white" />
          </div>
        )}
      </div>
    </main>
  )
}
