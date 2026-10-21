/** The path an old hash link such as `#/t/<id>` now has, if it is one. */
export function legacyPath(hash: string): string | undefined {
  if (!hash.startsWith('#/')) return undefined
  const [, page, id] = hash.slice(1).split('/')
  if (page === 't' && id) return `/t/${id}`
  if (page === 'mr' && id) return `/mr/${id}`
  if (page === 'mrs' || page === 'inbox') return `/${page}`
  return '/'
}
