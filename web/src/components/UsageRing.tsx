import { useQuery } from '@tanstack/react-query'
import { type AgentUsage, api } from '../api'
import { useNow } from '../now'
import { keys } from '../query'
import { agentKey } from '../rules'
import { hosted } from '../served'
import { resetsAt } from '../time'
import { Popover, cx } from '../ui'
import { GHOST_CHIP } from './ChipSelect'

/** Agents whose plan usage Silt reads, by `agentKey`; others aren't asked. */
const READ = ['claude', 'codex', 'copilot']

/** How full a limit is, as a colour: ok, then a warning from 75 %, then danger from 90 %. */
function tone(percent: number): string {
  return percent >= 90 ? 'text-danger' : percent >= 75 ? 'text-warn' : 'text-ok'
}

function Ring({ percent }: { percent: number }) {
  const radius = 6
  const around = 2 * Math.PI * radius
  return (
    <svg viewBox="0 0 16 16" className="h-4 w-4 -rotate-90" aria-hidden="true">
      <circle cx="8" cy="8" r={radius} fill="none" strokeWidth="2" className="stroke-line" />
      <circle
        cx="8"
        cy="8"
        r={radius}
        fill="none"
        strokeWidth="2"
        stroke="currentColor"
        strokeLinecap="round"
        strokeDasharray={around}
        strokeDashoffset={around * (1 - Math.min(100, Math.max(0, percent)) / 100)}
        className={tone(percent)}
      />
    </svg>
  )
}

export function UsagePanel({ usage, now }: { usage: AgentUsage; now: number }) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-baseline justify-between gap-4 text-xs">
        <span className="text-fg-2">Plan usage limits{usage.plan && ` · ${usage.plan}`}</span>
        <a href={usage.page} target="_blank" rel="noreferrer" className="shrink-0 text-fg-1 hover:underline">
          See all →
        </a>
      </div>
      {usage.limits.map((limit) => {
        const percent = Math.round(limit.used_percent)
        return (
          <div key={limit.name} className="flex flex-col gap-1.5">
            <div className="flex items-baseline justify-between gap-4 text-xs">
              <span className="text-fg-1">{limit.name}</span>
              <span className="text-fg-2 tabular-nums">
                {limit.resets_at_ms !== null && `${resetsAt(limit.resets_at_ms, now)} `}
                {percent}%
              </span>
            </div>
            <div
              role="meter"
              aria-label={limit.name}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={percent}
              className="h-1 overflow-hidden rounded-full bg-surface-3"
            >
              <div
                className={cx('h-full rounded-full bg-current', tone(limit.used_percent))}
                style={{ width: `${Math.min(100, limit.used_percent)}%` }}
              />
            </div>
          </div>
        )
      })}
      {usage.recorded && <p className="text-2xs text-fg-3">As {usage.agent} last recorded it; asking again failed.</p>}
    </div>
  )
}

/**
 * How much of its plan the agent has used, as a ring in the composer's row that fills with the fullest limit;
 * opened, each limit with when it resets (ADR 0095). Nothing shows where the agent has no plan limits to read.
 */
export function UsageRing({ agent, thread }: { agent: string; thread?: string }) {
  const now = useNow()
  const usage = useQuery({
    queryKey: keys.usage(agent, thread),
    queryFn: () => api.usage(agent, thread),
    enabled: READ.includes(agentKey(agent)) && !hosted(),
    // The daemon keeps an answer a minute, so this asks the provider at most that often.
    refetchInterval: (query) => (query.state.data ? 60_000 : false),
  })
  const data = usage.data
  if (!data || data.limits.length === 0) return null
  const fullest = Math.max(...data.limits.map((limit) => limit.used_percent))
  return (
    <Popover
      label="plan usage"
      triggerLabel={`plan usage: ${Math.round(fullest)}% of the fullest limit used`}
      triggerClassName={GHOST_CHIP}
      panelClassName="w-80 p-3"
      align="right"
      trigger={
        <span className="inline-flex items-center" title={`${data.agent}'s plan usage`}>
          <Ring percent={fullest} />
        </span>
      }
    >
      <UsagePanel usage={data} now={now} />
    </Popover>
  )
}
