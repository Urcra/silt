import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { api, short, type SearchForm, type SearchResult } from '../api'
import { keys, message } from '../query'
import { Button, Chip, Input, Tag } from '../ui'

const FILTERS: [Exclude<keyof SearchForm, 'status'>, string][] = [
  ['repo', 'repo'],
  ['author', 'author'],
  ['branch', 'branch'],
  ['file', 'file or dir'],
  ['since', 'since (YYYY-MM-DD)'],
]

const STATUSES = ['active', 'idle', 'merged'] as const

/** Hits by thread, in the order their best hit came. */
function byThread(hits: SearchResult[]): [string, SearchResult[]][] {
  const grouped = new Map<string, SearchResult[]>()
  for (const hit of hits) grouped.set(hit.thread, [...(grouped.get(hit.thread) ?? []), hit])
  return [...grouped.entries()]
}

export function Search() {
  const [query, setQuery] = useState('')
  const [filters, setFilters] = useState<SearchForm>({})
  const [results, setResults] = useState<SearchResult[]>()
  const [error, setError] = useState<string>()
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const titleOf = (thread: string) => threads.data?.find((summary) => summary.id === thread)?.title ?? short(thread)
  const active = Object.entries(filters).filter(([, value]) => value) as [keyof SearchForm, string][]

  async function run(event: React.FormEvent) {
    event.preventDefault()
    setError(undefined)
    try {
      setResults(await api.search(query, filters))
    } catch (failure) {
      setError(message(failure))
    }
  }

  return (
    <section className="mb-8">
      <form onSubmit={run} className="flex flex-wrap gap-2">
        <Input
          aria-label="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search conversations, including before compaction"
          className="min-w-64 flex-1"
        />
        <Button type="submit" variant="primary" icon="search">
          Search
        </Button>
        <div className="flex w-full flex-wrap items-center gap-2">
          {STATUSES.map((status) => (
            <Chip
              key={status}
              selected={filters.status === status}
              onClick={() => setFilters({ ...filters, status: filters.status === status ? undefined : status })}
            >
              {status}
            </Chip>
          ))}
          {FILTERS.map(([name, label]) => (
            <Input
              key={name}
              aria-label={name}
              placeholder={label}
              value={filters[name] ?? ''}
              onChange={(event) => setFilters({ ...filters, [name]: event.target.value })}
              className="w-36 text-xs"
            />
          ))}
        </div>
        {active.length > 0 && (
          <div aria-label="filters" className="flex w-full flex-wrap gap-1">
            {active.map(([name, value]) => (
              <Chip key={name} selected onClick={() => setFilters({ ...filters, [name]: undefined })} aria-label={`clear ${name}`}>
                {name}: {value} ×
              </Chip>
            ))}
          </div>
        )}
      </form>
      {error && <p className="mt-2 text-sm text-danger">{error}</p>}
      {results && (
        <div data-testid="search-results" className="mt-3 space-y-3 text-sm">
          {results.length === 0 && <p className="text-fg-3">No matches.</p>}
          {byThread(results).map(([thread, hits]) => (
            <section key={thread} aria-label={`hits in ${titleOf(thread)}`}>
              <h3 className="flex flex-wrap items-center gap-2">
                <Link className="font-medium text-accent-fg" to="/t/$id" params={{ id: thread }}>
                  {titleOf(thread)}
                </Link>
                <span className="text-xs text-fg-3">
                  {[hits[0].repo, hits[0].owner, hits[0].status].filter(Boolean).join(' · ')}
                </span>
              </h3>
              <ul className="mt-1 space-y-0.5 border-l border-line pl-3">
                {hits.map((hit, index) => (
                  <li key={index} className="flex flex-wrap items-baseline gap-2">
                    {hit.turn !== null && <Tag>turn {hit.turn}</Tag>}
                    {hit.before_compaction && <Tag tone="accent">before compaction</Tag>}
                    <span className="font-mono text-[11px] text-fg-3">{hit.kind}</span>
                    <span>{hit.snippet}</span>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}
    </section>
  )
}
