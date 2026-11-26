const MINUTE = 60_000
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR

/** How long ago `ms` was, as dense lists say it: `just now`, `4m`, `2h`, `yesterday`, `3d`, `3w`, or a date. */
export function ago(ms: number, now: number): string {
  const since = Math.max(0, now - ms)
  if (since < MINUTE) return 'just now'
  if (since < HOUR) return `${Math.floor(since / MINUTE)}m`
  if (since < DAY) return `${Math.floor(since / HOUR)}h`
  if (since < 2 * DAY) return 'yesterday'
  if (since < 7 * DAY) return `${Math.floor(since / DAY)}d`
  if (since < 30 * DAY) return `${Math.floor(since / (7 * DAY))}w`
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
}

/** Adds the sidebar's suffix to relative durations, never to localized dates. */
export function agoWithSuffix(ms: number, now: number): string {
  const said = ago(ms, now)
  return /^\d+[mhdw]$/.test(said) ? `${said} ago` : said
}

/** When a plan's limit resets, as Claude Code's usage says it: `Resets at 2:00 PM` today, `Resets Tue 7:00 AM` within the week, else `Resets Dec 1`. */
export function resetsAt(ms: number, now: number): string {
  const at = new Date(ms)
  const time = at.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })
  if (at.toDateString() === new Date(now).toDateString()) return `Resets at ${time}`
  if (ms - now < 6 * DAY) return `Resets ${at.toLocaleDateString(undefined, { weekday: 'short' })} ${time}`
  return `Resets ${at.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}`
}
