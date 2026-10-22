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
