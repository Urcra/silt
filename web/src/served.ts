// Where the app runs from (ADR 0026): a daemon serves it at `/`, the
// server at `/app/`, each writing its `<base href>` and who it is.

/** The path the app is served under, ending in `/`. */
export function basePath(): string {
  return new URL(document.baseURI).pathname
}

/** `path` (such as `/api/threads`) under the app's base. */
export function apiUrl(path: string): string {
  return basePath() + path.replace(/^\//, '')
}

/** A server as people tell servers apart: its host, from its URL. */
export function serverName(url: string): string {
  try {
    return new URL(url).host
  } catch {
    return url
  }
}

/** Whether the server hosts the app, so nothing on this machine runs agents for it. */
export function hosted(): boolean {
  return document.querySelector('meta[name="silt-served-by"]')?.getAttribute('content') === 'server'
}

/** Whether a daemon serves the app to a phone on its network (ADR 0112), rather than to its own machine. */
export function onLan(): boolean {
  return document.querySelector('meta[name="silt-served-by"]')?.getAttribute('content') === 'lan'
}
