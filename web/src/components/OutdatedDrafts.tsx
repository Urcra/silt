import { type Draft, type ReviewState, spanOf } from '../review'

/** Pending comments whose lines the thread has since deleted (ADR 0066): shown with the lines they were on, and sent
 * with the review as lines since deleted. */
export function OutdatedDrafts({ drafts, review }: { drafts: Draft[]; review: ReviewState }) {
  if (drafts.length === 0) return null
  return (
    <section aria-label="pending comments on lines since deleted" className="flex flex-col gap-2 rounded-md border border-line-subtle bg-accent-tint px-3 py-2">
      <p className="text-2xs text-fg-2">Their lines are gone since you wrote them; they go with the review as lines since deleted.</p>
      {drafts.map((draft) => (
        <div key={draft.id} role="group" aria-label="pending comment" className="flex flex-col gap-1 text-xs">
          <span className="font-mono text-2xs text-fg-2">{spanOf(draft.at)}</span>
          {draft.quote.length > 0 && (
            <pre className="overflow-x-auto rounded-sm bg-surface-1 px-2 py-1 font-mono text-2xs whitespace-pre text-fg-2">{draft.quote.join('\n')}</pre>
          )}
          <span className="flex items-start gap-2">
            <span className="rounded-sm border border-accent px-1 font-mono text-2xs text-accent-fg">Pending</span>
            <span className="min-w-0 flex-1 whitespace-pre-wrap">{draft.text}</span>
            <button type="button" className="text-danger" onClick={() => review.remove(draft.id)}>
              delete
            </button>
          </span>
        </div>
      ))}
    </section>
  )
}
