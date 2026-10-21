import { basePath, hosted } from './served'

/**
 * A link others can open to `path` within the app, such as `/t/<id>#turn-3`
 * (ADR 0031). From a machine on a server it points into the server's app,
 * since nobody else can open this machine's.
 */
export function permalink(path: string, server: string | null): string {
  const relative = path.replace(/^\//, '')
  if (hosted() || !server) return new URL(relative, document.baseURI).toString()
  return new URL(`app/${relative}`, server.endsWith('/') ? server : `${server}/`).toString()
}

/** `path` within the app, as a plain link's `href`. */
export function appHref(path: string): string {
  return basePath() + path.replace(/^\//, '')
}

export type LineRange = { start: number; end: number }

/** The lines a hash such as `#L4` or `#L4-L9` picks, in order. */
export function parseLines(hash: string): LineRange | null {
  const found = /^#?L(\d+)(?:-L(\d+))?$/.exec(hash)
  if (!found) return null
  const [first, last] = [Number(found[1]), Number(found[2] ?? found[1])]
  if (first < 1 || last < 1) return null
  return { start: Math.min(first, last), end: Math.max(first, last) }
}

export function linesHash({ start, end }: LineRange): string {
  return start === end ? `L${start}` : `L${start}-L${end}`
}

/** Where a file of a thread is shown, at version `at` when given. */
export function filePath(thread: string, path: string, at?: string, lines?: LineRange): string {
  const query = at ? `?at=${encodeURIComponent(at)}` : ''
  const hash = lines ? `#${linesHash(lines)}` : ''
  return `/t/${thread}/files/${path.split('/').map(encodeURIComponent).join('/')}${query}${hash}`
}

/** The new path of each file a unified diff's `diff --git` header starts. */
export function diffFile(line: string): string | null {
  if (!line.startsWith('diff --git ')) return null
  const at = line.lastIndexOf(' b/')
  return at < 0 ? null : line.slice(at + 3)
}
