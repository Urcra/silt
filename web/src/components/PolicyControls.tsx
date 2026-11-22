import type { Preset } from '../api'
import { PRESETS } from '../agentState'
import { KNOWN_AGENTS, PRESET_LABELS } from '../rules'
import { Icon, cx } from '../ui'
import { AgentMark } from './AgentMark'

/** The presets as a cap orders them, by how much goes through without asking (`silt_workspace::Preset::latitude`). */
const BY_LATITUDE: Preset[] = ['ask_for_everything', 'plan', 'ask_before_commands', 'auto', 'full_access']

/**
 * The most an agent may do without asking, as five steps (ADR 0114); Full access is no cap. A step another's rules
 * refuse stays shown, greyed, with why.
 */
export function PresetSteps({
  label,
  value,
  onChange,
  refused,
  disabled = false,
}: {
  label: string
  /** None for no cap. */
  value: Preset | null
  onChange: (cap: Preset | null) => void
  refused?: (preset: Preset) => string | undefined
  disabled?: boolean
}) {
  const cap = value ?? 'full_access'
  const chosen = PRESETS.find((preset) => preset.id === cap)
  return (
    <span className="flex min-w-0 flex-col gap-1">
      <span
        role="radiogroup"
        aria-label={label}
        className="inline-flex max-w-full self-start overflow-x-auto rounded-sm border border-line text-xs"
      >
        {BY_LATITUDE.map((id, index) => {
          const preset = PRESETS.find((candidate) => candidate.id === id)!
          const why = refused?.(id)
          return (
            <button
              key={id}
              type="button"
              role="radio"
              aria-checked={cap === id}
              aria-label={preset.label}
              title={why ?? `${preset.label}: ${preset.says}`}
              disabled={disabled || Boolean(why)}
              onClick={() => onChange(id === 'full_access' ? null : id)}
              className={cx(
                'inline-flex shrink-0 items-center gap-1.5 px-2.5 py-1 whitespace-nowrap max-md:min-h-11',
                index > 0 && 'border-l border-line',
                cap === id ? 'bg-accent-tint text-accent-fg' : 'text-fg-2 enabled:hover:bg-surface-2',
                why && 'cursor-not-allowed text-fg-3 line-through decoration-fg-3/60',
                disabled && !why && 'cursor-default',
              )}
            >
              {cap === id && <Icon name="lock" className="h-3 w-3" />}
              {preset.short}
            </button>
          )
        })}
      </span>
      {chosen && (
        <span className="text-2xs text-fg-2">
          {chosen.label}: {chosen.says}
        </span>
      )}
    </span>
  )
}

/** A cap in words, with the lock: `🔒 Ask before commands`, or Full access for none. */
export function CapText({ cap }: { cap: Preset | null | undefined }) {
  return (
    <span className="inline-flex items-center gap-1 whitespace-nowrap">
      <Icon name="lock" className="h-3 w-3 text-fg-3" />
      {cap ? PRESET_LABELS[cap] : 'Full access'}
    </span>
  )
}

/** The agents a rule allows, by their marks; their names are in the tooltip and for a screen reader. */
export function AgentMarks({ agents }: { agents: string[] | null | undefined }) {
  if (!agents) return <>Any</>
  if (agents.length === 0) return <>None</>
  const names = agents.map((key) => KNOWN_AGENTS.find((agent) => agent.key === key)?.name ?? key)
  return (
    <span role="img" aria-label={names.join(', ')} title={names.join(', ')} className="inline-flex items-center gap-1.5">
      {agents.map((key) => (
        <AgentMark key={key} agent={key} className="h-3.5 w-3.5" />
      ))}
    </span>
  )
}

/** Which of the agents Silt knows a rule allows, as chips with their marks to switch on and off. */
export function AgentToggles({
  label,
  value,
  onChange,
  refused,
  disabled = false,
}: {
  /** Says what each chip allows, before the agent's name: `allows`, or `acme allows`. */
  label: string
  value: string[]
  onChange: (agents: string[]) => void
  refused?: (key: string) => string | undefined
  disabled?: boolean
}) {
  return (
    <span className="flex flex-wrap gap-1.5">
      {KNOWN_AGENTS.map((agent) => {
        const on = value.includes(agent.key)
        const why = refused?.(agent.key)
        return (
          <button
            key={agent.key}
            type="button"
            role="checkbox"
            aria-checked={on}
            aria-label={`${label} ${agent.name}`}
            title={why}
            disabled={disabled}
            onClick={() => onChange(on ? value.filter((kept) => kept !== agent.key) : [...value, agent.key])}
            className={cx(
              'inline-flex h-7 items-center gap-1.5 rounded-sm border px-2 text-xs max-md:min-h-11',
              on ? 'border-accent bg-accent-tint text-fg-1' : 'border-line text-fg-2 enabled:hover:bg-surface-2',
              why && 'border-dashed text-fg-3',
              disabled && 'cursor-default opacity-60',
            )}
          >
            <AgentMark agent={agent.key} className={cx('h-3.5 w-3.5', why && 'opacity-50')} />
            {agent.name}
            {on && <Icon name="check" className="h-3 w-3 text-accent-fg" />}
          </button>
        )
      })}
    </span>
  )
}
