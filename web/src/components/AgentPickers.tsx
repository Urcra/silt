import type { Preset } from '../api'
import { PRESETS } from '../agentState'
import { Icon } from '../ui'
import { AgentMark } from './AgentMark'
import { ChipMenu } from './ChipMenu'

const sentence = (text: string) => text.charAt(0).toUpperCase() + text.slice(1)

/** The thread's permissions, the same five for every agent, first in the composer's row (ADR 0114). */
export function PermissionChip({
  value,
  onChange,
  refused,
  presets = PRESETS,
  ghost = true,
  label = 'permissions',
}: {
  value: Preset
  onChange: (preset: Preset) => void
  /** Why a preset can't be picked here: the workspace's rules, or an agent without the mode for it. */
  refused?: (preset: Preset) => string | undefined
  presets?: typeof PRESETS
  ghost?: boolean
  label?: string
}) {
  const chosen = PRESETS.find((preset) => preset.id === value)
  return (
    <ChipMenu
      label={label}
      value={value}
      ghost={ghost}
      heading="Permissions"
      title={chosen ? `${chosen.label}: ${chosen.says}` : undefined}
      trigger={
        <>
          <Icon name="lock" className="h-3.5 w-3.5 text-fg-3" />
          {chosen?.short ?? value}
        </>
      }
      options={presets.map((preset) => ({
        id: preset.id,
        name: preset.label,
        description: sentence(preset.says),
        refused: refused?.(preset.id),
      }))}
      onChange={(id) => onChange(id as Preset)}
      footer={ghost ? 'Shift+Tab steps through them' : undefined}
    />
  )
}

/** Which agent runs, by its mark; its name is in the tooltip and the list (ADR 0114). */
export function AgentChip({
  value,
  agents,
  onChange,
  refused,
  footer,
}: {
  value: string
  agents: { id: string; name: string; description?: string | null }[]
  onChange: (agent: string) => void
  refused?: (agent: string) => string | undefined
  footer?: string
}) {
  const chosen = agents.find((agent) => agent.id === value)
  return (
    <ChipMenu
      label="agent to start"
      value={value}
      heading="Agent"
      title={chosen ? `${chosen.name} · switch agent` : 'Pick an agent'}
      className="px-1.5"
      trigger={<AgentMark agent={value} name={chosen?.name} />}
      options={agents.map((agent) => ({
        id: agent.id,
        name: agent.name,
        description: agent.description,
        refused: refused?.(agent.id),
        lead: <AgentMark agent={agent.id} name={agent.name} className="h-[18px] w-[18px]" />,
      }))}
      onChange={onChange}
      footer={footer}
    />
  )
}

/**
 * How hard the agent thinks, as one bar per level of `effortLevel`, filled up to the chosen one. The agent's default
 * has no level, so no bars.
 */
export function EffortBars({ at, of }: { at: number | null; of: number }) {
  if (at === null) return null
  return (
    <span className="inline-flex h-[11px] items-end gap-[1.5px]" aria-hidden="true">
      {Array.from({ length: of }, (_, index) => (
        <i
          key={index}
          style={{ height: of === 1 ? 11 : 4 + (7 * index) / (of - 1) }}
          className={index <= at ? 'block w-[2.5px] rounded-[1px] bg-current' : 'block w-[2.5px] rounded-[1px] bg-line'}
        />
      ))}
    </span>
  )
}
