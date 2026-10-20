import { useEffect, useState } from 'react'

export type Route =
  | { page: 'threads' }
  | { page: 'thread'; id: string }
  | { page: 'mrs' }
  | { page: 'mr'; id: string }
  | { page: 'inbox' }

export function parseRoute(hash: string): Route {
  const [, page, id] = hash.replace(/^#/, '').split('/')
  if (page === 't' && id) return { page: 'thread', id: decodeURIComponent(id) }
  if (page === 'mrs') return { page: 'mrs' }
  if (page === 'inbox') return { page: 'inbox' }
  if (page === 'mr' && id) return { page: 'mr', id: decodeURIComponent(id) }
  return { page: 'threads' }
}

export function href(route: Route): string {
  switch (route.page) {
    case 'thread':
      return `#/t/${encodeURIComponent(route.id)}`
    case 'mr':
      return `#/mr/${encodeURIComponent(route.id)}`
    case 'mrs':
      return '#/mrs'
    case 'inbox':
      return '#/inbox'
    default:
      return '#/'
  }
}

export function useRoute(): Route {
  const [route, setRoute] = useState(() => parseRoute(window.location.hash))
  useEffect(() => {
    const update = () => setRoute(parseRoute(window.location.hash))
    window.addEventListener('hashchange', update)
    return () => window.removeEventListener('hashchange', update)
  }, [])
  return route
}
