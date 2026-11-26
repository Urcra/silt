import type { ReactNode } from 'react'
import type { AgentChoiceItem, Preset } from '../api'
import { PRESETS, SILT_AUTO_SAYS, ownModeSays } from '../agentState'
import { Icon } from '../ui'
import { AgentMark } from './AgentMark'
import { ChipMenu } from './ChipMenu'

const sentence = (text: string) => text.charAt(0).toUpperCase() + text.slice(1)

/** Where the permissions menu lists the agent's own modes, its ids are kept apart from the presets'. */
const MODE = 'mode:'

/**
 * The thread's permissions, the same five for every agent (ADR 0114), then the agent's own modes, such as Copilot's
 * Autopilot, under its name (ADR 0118). The chip shows the label alone, after the agent's mark in the row.
 */
export function PermissionChip({
  value,
  onChange,
  refused,
  presets = PRESETS,
  ghost = true,
  label = 'permissions',
  checked = false,
  judge,
  own,
}: {
  value: Preset
  onChange: (preset: Preset) => void
  /** Why a preset can't be picked here: the workspace's rules, or an agent without the mode for it. */
  refused?: (preset: Preset) => string | undefined
  presets?: typeof PRESETS
  ghost?: boolean
  label?: string
  /** Silt checks the agent's commands on Auto, as it has no auto mode of its own (ADR 0132). */
  checked?: boolean
  /** Under Auto, who checks for it, for a running thread's agent (ADR 0132). */
  judge?: ReactNode
  /** The agent's own modes, from `ownModes`, and the one it's in, if any. */
  own?: {
    agent: string
    name: string
    modes: AgentChoiceItem[]
    mode?: string
    onMode: (mode: string) => void
    /** Why the workspace refuses a mode. */
    refused?: (mode: string) => string | undefined
  }
}) {
  const chosen = PRESETS.find((preset) => preset.id === value)
  const inMode = own?.modes.find((mode) => mode.id === own.mode)
  const group = own && {
    key: own.agent,
    heading: (
      <>
        <AgentMark agent={own.agent} name={own.name} className="h-3 w-3" />
        {own.name}&apos;s own
      </>
    ),
  }
  return (
    <ChipMenu
      label={label}
      value={inMode ? MODE + inMode.id : value}
      ghost={ghost}
      heading="Permissions"
      title={inMode ? `${inMode.name}: ${ownModeSays(inMode)}` : chosen ? `${chosen.label}: ${chosen.says}` : undefined}
      trigger={inMode?.name ?? chosen?.short ?? value}
      options={[
        ...presets.map((preset) => ({
          id: preset.id,
          name: preset.label,
          description: sentence(preset.id === 'auto' && checked ? SILT_AUTO_SAYS : preset.says),
          refused: refused?.(preset.id),
        })),
        ...(own?.modes.map((mode) => ({
          id: MODE + mode.id,
          name: mode.name,
          description: sentence(ownModeSays(mode)),
          refused: own.refused?.(mode.id),
          group,
        })) ?? []),
      ]}
      onChange={(id) => (id.startsWith(MODE) ? own?.onMode(id.slice(MODE.length)) : onChange(id as Preset))}
      footer={ghost ? 'Shift+Tab steps through them' : undefined}
      extra={judge ? { after: 'auto', node: judge } : undefined}
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
  onStop,
}: {
  value: string
  agents: { id: string; name: string; description?: string | null }[]
  onChange: (agent: string) => void
  refused?: (agent: string) => string | undefined
  footer?: string
  /** Where the agent runs, the menu ends with stopping it. */
  onStop?: () => void
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
      action={
        onStop && {
          label: 'Stop agent',
          title: 'Stops the agent; the thread keeps everything it did',
          lead: (
            <span className="flex h-[18px] w-[18px] items-center justify-center text-fg-2">
              <Icon name="stop" className="h-3.5 w-3.5" />
            </span>
          ),
          run: onStop,
        }
      }
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
