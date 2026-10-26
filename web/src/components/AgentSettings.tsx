import { Fragment, type ReactNode } from 'react'
import type { AgentChoiceItem, AgentSetting } from '../api'
import { Icon, Popover } from '../ui'
import { ChipSelect, GHOST_CHIP } from './ChipSelect'

/**
 * An agent's mode and settings as both composers show them, in one row: the
 * mode and the model as chips, everything else behind one settings chip. On a
 * phone that one chip, named by the model, holds them all.
 */
export function AgentSettings({
  modes,
  onMode,
  settings,
  value = (setting) => setting.current,
  onSetting,
  more,
  narrow,
}: {
  modes?: { current: string; available: AgentChoiceItem[] } | null
  onMode?: (mode: string) => void
  settings: AgentSetting[]
  /** What a setting shows as chosen, where the composer keeps its own choice. */
  value?: (setting: AgentSetting) => string
  onSetting: (id: string, value: string) => void
  /** Anything else of the agent's for the settings chip, such as stopping it. */
  more?: ReactNode
  narrow: boolean
}) {
  // An agent may list its mode among its settings too, which the Mode chip already sets.
  const shown = settings.filter((setting) => !(modes && setting.category === 'mode'))
  // By its tag, or by its id where the agent tags none.
  const isModel = (setting: AgentSetting) => setting.category === 'model' || (setting.category === null && setting.id === 'model')
  const model = shown.find(isModel)
  const rest = shown.filter((setting) => setting !== model)
  const chip = (setting: AgentSetting) => (
    <ChipSelect
      ghost
      key={setting.id}
      label={setting.name.toLowerCase()}
      // A model names itself; "High" or "Off" need saying what they set.
      prefix={isModel(setting) ? undefined : setting.name}
      value={value(setting)}
      onChange={(chosen) => onSetting(setting.id, chosen)}
      options={setting.choices}
    />
  )
  const modeChip = modes && onMode && (
    <ChipSelect
      ghost
      label="mode"
      prefix="Mode"
      value={modes.current}
      onChange={onMode}
      options={modes.available.map((mode) => ({ id: mode.id, name: mode.name, description: mode.description }))}
    />
  )
  const inPopover = narrow ? [modeChip, ...shown.map(chip), more] : [...rest.map(chip), more]
  const name = model?.choices.find((choice) => choice.id === value(model))?.name
  return (
    <>
      {!narrow && modeChip}
      {!narrow && model && chip(model)}
      {inPopover.some(Boolean) && (
        <Popover
          label="agent settings"
          triggerLabel="agent settings"
          triggerClassName={GHOST_CHIP}
          panelClassName="w-72 p-2"
          trigger={
            <span className="inline-flex items-center gap-1 text-fg-2">
              <Icon name="settings" className="h-3.5 w-3.5" />
              {narrow ? (name ?? 'Settings') : <span className="sr-only">More settings</span>}
            </span>
          }
        >
          <div className="flex flex-col items-start gap-1.5">
            {inPopover.map((node, index) => node && <Fragment key={index}>{node}</Fragment>)}
          </div>
        </Popover>
      )}
    </>
  )
}
