import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate, useSearch } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { api, short, type SearchForm, type SearchResult } from '../api'
import { projectName } from '../projects'
import { keys, message } from '../query'
import { threadTone } from '../threadLook'
import { Button, Dot, EmptyState, Icon, Input, Kbd, Skeleton, cx } from '../ui'
import { Mock } from './Mock'
import { Page } from './Page'
import { Snippet } from './Snippet'

const FILTERS: [Exclude<keyof SearchForm, 'status'>, string][] = [
  ['repo', 'repo'],
  ['author', 'author'],
  ['branch', 'branch'],
  ['file', 'file or dir'],
  ['since', 'since'],
]

const STATUSES = ['active', 'idle', 'merged'] as const

/** What said a hit, as its label reads. */
const ROLE: Record<string, string> = {
  user: 'prompt',
  assistant: 'assistant',
  thinking: 'thinking',
  tool_call: 'tool call',
  tool_result: 'tool output',
  compaction: 'summary',
  comment: 'comment',
}

/** Hits by thread, in the order their best hit came. */
function byThread(hits: SearchResult[]): [string, SearchResult[]][] {
  const grouped = new Map<string, SearchResult[]>()
  for (const hit of hits) grouped.set(hit.thread, [...(grouped.get(hit.thread) ?? []), hit])
  return [...grouped.entries()]
}

const SHOWN = 3

function ThreadHits({
  thread,
  hits,
  title,
  project,
  tone,
}: {
  thread: string
  hits: SearchResult[]
  title: string
  /** The thread's project, which hits found on this machine don't carry. */
  project: string | undefined
  tone: ReturnType<typeof threadTone>
}) {
  const [all, setAll] = useState(false)
  const shown = all ? hits : hits.slice(0, SHOWN)
  return (
    <section aria-label={`hits in ${title}`} className="overflow-hidden rounded-md border border-line bg-elevated">
      <header className="flex flex-wrap items-center gap-2 border-b border-line-subtle px-3 py-2">
        <Dot tone={tone} />
        <Link className="font-medium text-fg-1 hover:text-accent-fg" to="/t/$id" params={{ id: thread }}>
          {title}
        </Link>
        <span className="font-mono text-2xs text-fg-2">
          <span data-volatile>{short(thread)}</span>
          {[projectName(hits[0].repo ?? project ?? ''), hits[0].owner, hits[0].status].filter(Boolean).map((part) => ` · ${part}`)}
        </span>
        <span className="ml-auto text-xs text-fg-2">
          {hits.length} {hits.length === 1 ? 'hit' : 'hits'}
        </span>
      </header>
      <ul className="flex flex-col">
        {shown.map((hit, index) => (
          <li key={index} className="grid grid-cols-[64px_84px_minmax(0,1fr)] items-baseline gap-3 border-b border-line-subtle px-3 py-2 text-sm last:border-b-0 max-md:grid-cols-[56px_minmax(0,1fr)]">
            {hit.turn !== null ? (
              <Link
                className="font-mono text-2xs text-accent-fg max-md:inline-flex max-md:min-h-11 max-md:items-center"
                to="/t/$id"
                params={{ id: thread }}
                hash={`turn-${hit.turn}`}
              >
                turn {hit.turn}
              </Link>
            ) : (
              <span className="font-mono text-2xs text-fg-2">—</span>
            )}
            <span className="font-mono text-2xs tracking-[0.04em] text-fg-2 uppercase max-md:hidden">{ROLE[hit.kind] ?? hit.kind}</span>
            <span className={cx('flex min-w-0 flex-col gap-0.5', hit.kind.startsWith('tool') && 'font-mono text-xs')}>
              <span>
                <Snippet text={hit.snippet} />
              </span>
              {hit.before_compaction && (
                <span className="inline-flex items-center gap-1 font-sans text-2xs text-fg-2">
                  <Icon name="clock" className="h-3 w-3" />
                  before compaction
                </span>
              )}
            </span>
          </li>
        ))}
        {hits.length > SHOWN && (
          <li className="px-3 py-1.5">
            <button type="button" className="text-xs text-accent-fg hover:text-fg-1 max-md:min-h-11" onClick={() => setAll(!all)}>
              {all ? 'Show fewer' : `${hits.length - SHOWN} more in this thread`}
            </button>
          </li>
        )}
      </ul>
    </section>
  )
}

/** Search across every thread, including what came before compaction (the Search board). */
export function Search() {
  const asked = useSearch({ strict: false }) as { q?: string }
  const [query, setQuery] = useState(asked.q ?? '')
  const [filters, setFilters] = useState<SearchForm>({})
  const [adding, setAdding] = useState<keyof SearchForm | null>(null)
  const [results, setResults] = useState<SearchResult[]>()
  const [searching, setSearching] = useState(false)
  const [error, setError] = useState<string>()
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const summaryOf = (thread: string) => threads.data?.find((summary) => summary.id === thread)
  const active = Object.entries(filters).filter(([, value]) => value) as [keyof SearchForm, string][]

  const navigate = useNavigate()
  const searched = useRef<string | undefined>(undefined)
  const run = async (text = query, using = filters) => {
    setError(undefined)
    setSearching(true)
    // The address keeps the words, so a reload or a shared link searches again.
    searched.current = text
    if (text !== asked.q) void navigate({ to: '/search', search: { q: text || undefined }, replace: true })
    try {
      setResults(await api.search(text, using))
    } catch (failure) {
      setError(message(failure))
    } finally {
      setSearching(false)
    }
  }
  useEffect(() => {
    if (asked.q && asked.q !== searched.current) void run(asked.q, {})
    // Only what the address asked for, once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [asked.q])

  const grouped = results ? byThread(results) : []
  const early = results?.filter((hit) => hit.before_compaction).length ?? 0
  const setFilter = (name: keyof SearchForm, value: string | undefined) => {
    const next = { ...filters, [name]: value }
    setFilters(next)
    if (results) void run(query, next)
  }

  return (
    <Page flush label="search" title="Search" meta="everything any agent said, before and after compaction">
      <div className="mx-auto flex w-full max-w-[1200px] flex-col gap-6 px-4 py-5 md:px-6 lg:flex-row">
        <div className="flex min-w-0 flex-1 flex-col gap-4">
          <form
            role="search"
            className="flex flex-col gap-2.5"
            onSubmit={(event) => {
              event.preventDefault()
              void run()
            }}
          >
            <label htmlFor="search-query" className="sr-only">
              Search
            </label>
            <div className="flex gap-2">
              <div className="relative flex min-w-0 flex-1 items-center">
                <Input
                  id="search-query"
                  aria-label="search"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="Conversations, prompts and comments, including before compaction"
                  className="w-full py-2 pr-10 text-base"
                />
                <span className="absolute right-2">
                  <Kbd>⏎</Kbd>
                </span>
              </div>
              <Button type="submit" variant="primary" icon="search" className="max-md:hidden">
                Search
              </Button>
            </div>
            <div aria-label="filters" className="flex flex-wrap items-center gap-1.5">
              {active.map(([name, value]) => (
                <Button key={name} size="sm" aria-label={`clear ${name}`} onClick={() => setFilter(name, undefined)}>
                  <span className="font-mono">
                    {name}:{value}
                  </span>
                  <span aria-hidden="true" className="text-fg-2">
                    ×
                  </span>
                </Button>
              ))}
              {!filters.status &&
                STATUSES.map((status) => (
                  <Button
                    key={status}
                    variant="ghost"
                    size="sm"
                    aria-pressed={false}
                    className="border-dashed border-line"
                    onClick={() => setFilter('status', status)}
                  >
                    {status}
                  </Button>
                ))}
              {FILTERS.filter(([name]) => !filters[name]).map(([name, label]) =>
                adding === name ? (
                  <form
                    key={name}
                    className="flex items-center gap-1"
                    onSubmit={(event) => {
                      event.preventDefault()
                      const value = new FormData(event.currentTarget).get('value')
                      setAdding(null)
                      if (typeof value === 'string' && value.trim()) setFilter(name, value.trim())
                    }}
                  >
                    <Input
                      name="value"
                      aria-label={name}
                      autoFocus
                      type={name === 'since' ? 'date' : 'text'}
                      placeholder={label}
                      className="w-36 text-xs"
                      onBlur={(event) => {
                        if (!event.target.value) setAdding(null)
                      }}
                    />
                  </form>
                ) : (
                  <Button key={name} variant="ghost" size="sm" className="border-dashed border-line" onClick={() => setAdding(name)}>
                    + {label}
                  </Button>
                ),
              )}
              {results && (
                <span className="ml-auto text-xs text-fg-2">
                  {results.length} {results.length === 1 ? 'hit' : 'hits'} in {grouped.length} {grouped.length === 1 ? 'thread' : 'threads'}
                  {early > 0 && ` · ${early} from before compaction`}
                </span>
              )}
            </div>
          </form>
          {error && <p className="text-sm text-danger">{error}</p>}
          {searching && <Skeleton className="h-24" />}
          {!results && !searching && (
            <EmptyState title="Search what was said, in every thread">
              Prompts, the agents’ answers, their tool calls and comments, kept even after an agent compacted its context.
            </EmptyState>
          )}
          {results && (
            <div data-testid="search-results" className="flex flex-col gap-3">
              {results.length === 0 && <EmptyState title="No matches">Try fewer words, or take a filter off.</EmptyState>}
              {grouped.map(([thread, hits]) => {
                const summary = summaryOf(thread)
                return (
                  <ThreadHits
                    key={thread}
                    thread={thread}
                    hits={hits}
                    title={summary?.title ?? short(thread)}
                    project={summary?.repo}
                    tone={summary ? threadTone(summary) : 'idle'}
                  />
                )
              })}
            </div>
          )}
        </div>
        <Mock id="search.saved">
        <aside aria-label="saved searches" className="flex shrink-0 flex-col gap-1.5 lg:w-56">
          <h2 className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">Saved</h2>
            {[
              { label: 'author:alice status:active', filters: { author: 'alice', status: 'active' } as SearchForm },
              { label: 'status:merged', filters: { status: 'merged' } as SearchForm },
            ].map((saved) => (
              <button
                key={saved.label}
                type="button"
                className="rounded-sm px-2 py-1 text-left font-mono text-2xs text-fg-1 hover:bg-surface-2 max-md:min-h-11"
                onClick={() => {
                  setFilters(saved.filters)
                  void run(query, saved.filters)
                }}
              >
                {saved.label}
              </button>
            ))}
        </aside>
        </Mock>
      </div>
    </Page>
  )
}
