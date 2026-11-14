import { Fragment, type ReactNode, useContext } from 'react'
import { presetDriven } from '../agentState'
import type { AgentChoiceItem, AgentSetting } from '../api'
import { Icon, Popover } from '../ui'
import { ChipSelect, GHOST_CHIP } from './ChipSelect'
import { ComposerFolded } from './composerFolded'

/** By its tag, or by its id where the agent tags none. */
function isModel(setting: AgentSetting): boolean {
  return setting.category === 'model' || (setting.category === null && setting.id === 'model')
}

/** How hard the agent thinks, by its tag or a name agents give it. */
function isEffort(setting: AgentSetting): boolean {
  return setting.category === 'thought_level' || /^(effort|reasoning|thinking)/i.test(setting.id)
}

/**
 * An agent's mode and settings as both composers show them, in one row: the
 * mode, the model and how hard it thinks as chips, everything else behind one
 * menu. On a phone, or where the row has no room for them, that menu, named
 * by the model, holds them all.
 */
export function AgentSettings({
  modes,
  mode,
  onMode,
  settings,
  value = (setting) => setting.current,
  onSetting,
  more,
  narrow,
  refusedMode,
}: {
  modes?: AgentChoiceItem[] | null
  /** The mode chosen, of `modes`. */
  mode?: string
  onMode?: (mode: string) => void
  settings: AgentSetting[]
  /** What a setting shows as chosen, where the composer keeps its own choice. */
  value?: (setting: AgentSetting) => string
  onSetting: (id: string, value: string) => void
  /** Anything else of the agent's for the menu, such as stopping it. */
  more?: ReactNode
  narrow: boolean
  /** Why the workspace's rules refuse a mode, which is then greyed out (ADR 0064). */
  refusedMode?: (mode: string) => string | undefined
}) {
  const roomless = useContext(ComposerFolded)
  const folded = narrow || roomless
  // An agent may list its mode among its settings too, which the Mode chip already sets, and the preset sets its own
  // switch for asking nothing.
  const shown = settings.filter((setting) => !(modes && setting.category === 'mode') && !presetDriven(setting))
  const inline = shown.filter((setting) => isModel(setting) || isEffort(setting))
  const rest = shown.filter((setting) => !inline.includes(setting))
  const chip = (setting: AgentSetting) => (
    <ChipSelect
      ghost
      key={setting.id}
      label={setting.name.toLowerCase()}
      // A model names itself; "High" or "Off" need saying what they set, which for effort is one word whatever
      // the agent calls it ("Reasoning Effort", "Thinking").
      prefix={isModel(setting) ? undefined : isEffort(setting) ? 'Effort' : setting.name}
      value={value(setting)}
      onChange={(chosen) => onSetting(setting.id, chosen)}
      options={setting.choices}
    />
  )
  const modeChip = modes && modes.length > 0 && mode !== undefined && onMode && (
    <ChipSelect
      ghost
      label="mode"
      prefix="Mode"
      value={mode}
      onChange={onMode}
      options={modes.map((choice) => ({ id: choice.id, name: choice.name, description: choice.description, refused: refusedMode?.(choice.id) }))}
    />
  )
  const inMenu = folded ? [modeChip, ...shown.map(chip), more] : [...rest.map(chip), more]
  const model = shown.find(isModel)
  const modelName = model?.choices.find((choice) => choice.id === value(model))?.name
  return (
    <>
      {!folded && modeChip}
      {!folded && inline.map(chip)}
      {inMenu.some(Boolean) && (
        <Popover
          label="agent settings"
          triggerLabel="agent settings"
          triggerClassName={GHOST_CHIP}
          panelClassName="w-72 p-2"
          trigger={
            <span className="inline-flex items-center gap-1 text-fg-2" title="More of the agent's settings">
              <Icon name={folded ? 'settings' : 'more'} className="h-3.5 w-3.5" />
              {folded && (modelName ?? 'Settings')}
            </span>
          }
        >
          <div className="flex flex-col items-start gap-1.5">
            {inMenu.map((node, index) => node && <Fragment key={index}>{node}</Fragment>)}
          </div>
        </Popover>
      )}
    </>
  )
}
