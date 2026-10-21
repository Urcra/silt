import { useState } from 'react'
import { api, short, type SearchForm, type SearchResult } from '../api'
import { Link } from '@tanstack/react-router'
import { Button, Input } from '../ui'

const FILTERS: [keyof SearchForm, string][] = [
  ['repo', 'repo'],
  ['author', 'author'],
  ['status', 'status'],
  ['branch', 'branch'],
  ['file', 'file or dir'],
  ['since', 'since (YYYY-MM-DD)'],
]

export function Search() {
  const [query, setQuery] = useState('')
  const [filters, setFilters] = useState<SearchForm>({})
  const [results, setResults] = useState<SearchResult[]>()
  const [error, setError] = useState<string>()

  async function run(event: React.FormEvent) {
    event.preventDefault()
    setError(undefined)
    try {
      setResults(await api.search(query, filters))
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure))
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
        <div className="flex w-full flex-wrap gap-2">
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
      </form>
      {error && <p className="mt-2 text-sm text-danger">{error}</p>}
      {results && (
        <ul data-testid="search-results" className="mt-3 space-y-1 text-sm">
          {results.length === 0 && <li className="text-fg-3">No matches.</li>}
          {results.map((hit, index) => (
            <li key={index}>
              <Link className="font-mono text-accent-fg" to="/t/$id" params={{ id: hit.thread }}>
                {short(hit.thread)}
              </Link>{' '}
              <span className="text-xs text-fg-3">
                {[hit.owner, hit.status, hit.kind].filter(Boolean).join(' · ')}
              </span>{' '}
              {hit.snippet}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
