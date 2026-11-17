import type { DesignWhere } from './api'

/** Where a design system's page is, as its address names it (ADR 0088). */
export type DesignSearch = DesignWhere

export function designSearch(search: Record<string, unknown>): DesignSearch {
  return {
    source: search.source === 'personal' ? 'personal' : 'project',
    name: typeof search.name === 'string' ? search.name : '',
    ...(typeof search.clone === 'string' ? { clone: search.clone } : {}),
    ...(typeof search.thread === 'string' ? { thread: search.thread } : {}),
  }
}
