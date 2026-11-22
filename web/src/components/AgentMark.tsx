import { AGENT_LOGOS } from '../agentLogos'
import { agentKey, agentName } from '../rules'
import { cx } from '../ui'

/**
 * An agent shown by its mark rather than its name (ADR 0114): its logo where Silt has one, Silt's own for the local
 * agent, else its initial. `agent` is its id, key or command; `name` what it's called where that's known better.
 * Decorative unless `labelled`, as most places say the name beside it or in their own label.
 */
export function AgentMark({
  agent,
  name,
  className = 'h-4 w-4',
  labelled = false,
}: {
  agent: string
  name?: string
  className?: string
  labelled?: boolean
}) {
  const key = agentKey(agent)
  const called = name || agentName(agent)
  const a11y = labelled ? { role: 'img', 'aria-label': called } : { 'aria-hidden': true }
  const paths = AGENT_LOGOS[key]
  if (paths) {
    return (
      <svg viewBox="0 0 24 24" fill="currentColor" fillRule="evenodd" className={cx('shrink-0', className)} {...a11y}>
        {paths.map((d) => (
          <path key={d} d={d} clipRule="evenodd" />
        ))}
      </svg>
    )
  }
  if (key === 'local') {
    return (
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
        className={cx('shrink-0', className)}
        {...a11y}
      >
        <path d="M3.5 7.25c3-2.25 6 2.25 9 0s4.5-1.5 6 0M3.5 12c3-2.25 6 2.25 9 0s4.5-1.5 6 0M3.5 16.75c3-2.25 6 2.25 9 0s4.5-1.5 6 0" />
      </svg>
    )
  }
  return (
    <span
      className={cx(
        'inline-flex shrink-0 items-center justify-center rounded-[25%] border border-current text-[0.6em] leading-none font-semibold',
        className,
      )}
      {...a11y}
    >
      {initial(called)}
    </span>
  )
}

function initial(name: string): string {
  return (name.match(/[\p{L}\p{N}]/u)?.[0] ?? '?').toUpperCase()
}
