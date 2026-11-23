import { Fragment, type ReactNode, useContext } from 'react'
import { permissionModes, presetDriven } from '../agentState'
import type { AgentChoiceItem, AgentSetting } from '../api'
import { effortLevel, isEffort, isModel } from '../agentSettings'
import { Icon, Popover, cx } from '../ui'
import { EffortBars } from './AgentPickers'
import { ChipSelect, GHOST_CHIP } from './ChipSelect'
import { ComposerFolded } from './composerFolded'

/**
 * An agent's settings as both composers show them, in one row (ADR 0114): a mode where its modes say how it works
 * rather than what it asks, then its model and how hard it thinks in one chip, everything else behind one menu. On a
 * phone, or where the row has no room for them, that menu, named by the model, holds them all. Modes that are
 * permissions don't show: the permissions chip moves the agent between them.
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
  /** Modes that aren't permissions, such as OpenCode's Build and Plan. */
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
  /** Why the workspace's rules refuse a mode, which is then greyed out (ADR 0065). */
  refusedMode?: (mode: string) => string | undefined
}) {
  const roomless = useContext(ComposerFolded)
  const folded = narrow || roomless
  // An agent may list its mode among its settings too, which the mode chip or the preset already sets, and the preset
  // sets its own switch for asking nothing.
  const shown = settings.filter(
    (setting) => !(setting.category === 'mode' && (modes?.length || permissionModes(setting.choices))) && !presetDriven(setting),
  )
  const model = shown.find(isModel)
  const effort = shown.find(isEffort)
  const rest = shown.filter((setting) => setting !== model && setting !== effort)
  const chip = (setting: AgentSetting) => (
    <ChipSelect
      ghost
      key={setting.id}
      label={setting.name.toLowerCase()}
      prefix={isModel(setting) ? undefined : isEffort(setting) ? 'Effort' : setting.name}
      value={value(setting)}
      onChange={(chosen) => onSetting(setting.id, chosen)}
      options={setting.choices}
    />
  )
  const modeChip = modes && modes.length > 0 && mode !== undefined && onMode && (
    <ChipSelect
      ghost
      chevron={false}
      label="mode"
      value={mode}
      onChange={onMode}
      options={modes.map((choice) => ({
        id: choice.id,
        name: choice.name,
        description: choice.description,
        refused: refusedMode?.(choice.id),
      }))}
    />
  )
  const modelName = model?.choices.find((choice) => choice.id === value(model))?.name
  const effortName = effort?.choices.find((choice) => choice.id === value(effort))?.name
  const level = effort && effortLevel(effort.choices, value(effort))
  const bars = level && level.at !== null && <EffortBars at={level.at} of={level.of} />
  const named = [modelName, effortName && `${effortName} effort`].filter(Boolean).join(' · ')
  const inMenu = folded ? [modeChip, ...shown.map(chip), more] : [...rest.map(chip), more]
  return (
    <>
      {!folded && modeChip}
      {!folded && (model || effort) && (
        <Popover
          label="model and effort"
          triggerLabel={named ? `model and effort: ${named}` : 'model and effort'}
          triggerTitle={named}
          triggerClassName={cx(GHOST_CHIP, 'aria-expanded:bg-surface-2')}
          // As wide as the effort steps need, so every step's name fits on one row.
          panelClassName="w-max min-w-72 max-w-[calc(100vw-1rem)] p-1"
          trigger={
            <span className="inline-flex items-center gap-1.5 text-fg-1">
              {modelName ?? 'Effort'}
              {bars && <span className="text-fg-2">{bars}</span>}
            </span>
          }
        >
          {(close) => (
            <ModelAndEffort
              model={model}
              effort={effort}
              value={value}
              onSetting={(id, chosen, done) => {
                onSetting(id, chosen)
                if (done) close()
              }}
            />
          )}
        </Popover>
      )}
      {inMenu.some(Boolean) && (
        <Popover
          label="agent settings"
          triggerLabel="agent settings"
          triggerClassName={GHOST_CHIP}
          panelClassName="w-72 p-2"
          trigger={
            <span className="inline-flex items-center gap-1.5 text-fg-2" title="More of the agent's settings">
              <Icon name={folded ? 'settings' : 'more'} className="h-3.5 w-3.5" />
              {folded && <span className="text-fg-1">{modelName ?? 'Settings'}</span>}
              {folded && bars}
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

/** Beyond this many steps of effort, the steps show their names alone so the row stays a panel's width. */
const MAX_STEPS_WITH_BARS = 6

/** The model chip's panel: the agent's models to pick one, then how hard it thinks, in steps. */
function ModelAndEffort({
  model,
  effort,
  value,
  onSetting,
}: {
  model?: AgentSetting
  effort?: AgentSetting
  value: (setting: AgentSetting) => string
  /** `done` closes the panel, as picking a model does; a step of effort leaves it open. */
  onSetting: (id: string, value: string, done: boolean) => void
}) {
  return (
    <div className="flex flex-col">
      {model && (
        // Sized by the panel, not by its descriptions, which wrap.
        <div role="menu" aria-label="model" className="flex w-0 min-w-full flex-col">
          <span className="px-2 pt-1.5 pb-1 text-2xs tracking-[0.06em] text-fg-3 uppercase">Model</span>
          {/* Its own scroll, so a long list of models keeps the effort steps in view. */}
          <div className="flex max-h-[min(18rem,45dvh)] flex-col overflow-y-auto">
            {model.choices.map((choice) => {
              const checked = choice.id === value(model)
              return (
                <button
                  key={choice.id}
                  type="button"
                  role="menuitemradio"
                  aria-checked={checked}
                  onClick={() => onSetting(model.id, choice.id, true)}
                  className={cx(
                    'flex w-full items-center gap-2.5 rounded-sm px-2 py-1.5 text-left text-sm text-fg-1 hover:bg-surface-2 focus:bg-surface-2 focus:outline-none max-md:min-h-11',
                    checked && 'bg-surface-2',
                  )}
                >
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="truncate">{choice.name}</span>
                    {choice.description && <span className="text-xs text-fg-3">{choice.description}</span>}
                  </span>
                  {checked ? (
                    <Icon name="check" className="h-3.5 w-3.5 shrink-0 text-accent-fg" />
                  ) : (
                    <span className="h-3.5 w-3.5 shrink-0" />
                  )}
                </button>
              )
            })}
          </div>
        </div>
      )}
      {effort && (
        <>
          {model && <div className="my-1 h-px bg-line-subtle" />}
          <span className="px-2 pt-1 pb-1 text-2xs tracking-[0.06em] text-fg-3 uppercase">Effort</span>
          <div
            role="radiogroup"
            aria-label="effort"
            className="mx-1.5 mb-1.5 flex flex-wrap overflow-hidden rounded-sm border border-line"
          >
            {effort.choices.map((choice, index) => {
              const checked = choice.id === value(effort)
              const { at, of } = effortLevel(effort.choices, choice.id)
              return (
                <button
                  key={choice.id}
                  type="button"
                  role="radio"
                  aria-checked={checked}
                  title={choice.description ?? undefined}
                  onClick={() => onSetting(effort.id, choice.id, false)}
                  className={cx(
                    'inline-flex h-7 flex-auto items-center justify-center gap-1.5 px-2 text-xs whitespace-nowrap max-md:min-h-11',
                    index > 0 && 'border-l border-line',
                    checked ? 'bg-accent-tint text-accent-fg' : 'text-fg-2 hover:bg-surface-2',
                  )}
                >
                  {effort.choices.length <= MAX_STEPS_WITH_BARS && <EffortBars at={at} of={of} />}
                  {choice.name}
                </button>
              )
            })}
          </div>
        </>
      )}
    </div>
  )
}
