import { Fragment, type KeyboardEvent, type ReactNode, useEffect, useRef } from 'react'
import { Icon, Popover, cx } from '../ui'
import { GHOST_CHIP } from './ChipSelect'

export interface ChipChoice {
  id: string
  name: string
  description?: string | null
  /** Why it can't be picked here; it stays listed, greyed, with this under its name (ADR 0065). */
  refused?: string
  /** Before the name, such as the agent's mark. */
  lead?: ReactNode
  /** Over the first choice of a group that follows others, such as the agent's own modes after the presets (ADR 0118). */
  group?: { key: string; heading: ReactNode }
}

export interface ChipAction {
  label: string
  title?: string
  lead?: ReactNode
  run: () => void
}

/**
 * A chip in the composer's row that opens a list to pick one from: each choice's name with what it does under it, a
 * refused one greyed with why (ADR 0114). It floats over the page, so the row's scrolling doesn't clip it.
 */
export function ChipMenu({
  label,
  value,
  options,
  onChange,
  trigger,
  title,
  heading,
  footer,
  action,
  ghost = true,
  className,
}: {
  /** What it picks, for a screen reader; also the trigger's name. */
  label: string
  value: string
  options: ChipChoice[]
  onChange: (id: string) => void
  /** What the chip shows. */
  trigger: ReactNode
  title?: string
  /** Over the list, as `Permissions`. */
  heading?: string
  footer?: ReactNode
  /** Under everything, something to do rather than pick, such as stopping the agent. */
  action?: ChipAction
  /** Borderless until hovered, in the composer's row; else bordered, as in a form. */
  ghost?: boolean
  className?: string
}) {
  return (
    <Popover
      label={label}
      triggerLabel={label}
      triggerTitle={title}
      triggerValue={value}
      trigger={trigger}
      triggerClassName={cx(
        ghost
          ? GHOST_CHIP
          : 'inline-flex h-7 items-center gap-1.5 rounded-sm border border-line bg-elevated px-2 text-xs text-fg-1 hover:bg-surface-2 max-md:min-h-11',
        'aria-expanded:bg-surface-2',
        className,
      )}
      panelClassName="w-72 p-1"
    >
      {(close) => (
        <ChoiceList
          label={label}
          value={value}
          options={options}
          heading={heading}
          footer={footer}
          action={
            action && {
              ...action,
              run: () => {
                close()
                action.run()
              },
            }
          }
          onPick={(id) => {
            close()
            onChange(id)
          }}
        />
      )}
    </Popover>
  )
}

function ChoiceList({
  label,
  value,
  options,
  heading,
  footer,
  action,
  onPick,
}: {
  label: string
  value: string
  options: ChipChoice[]
  heading?: string
  footer?: ReactNode
  action?: ChipAction
  onPick: (id: string) => void
}) {
  const list = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const items = list.current?.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]:not([aria-disabled="true"])')
    const chosen = list.current?.querySelector<HTMLButtonElement>('[aria-checked="true"]:not([aria-disabled="true"])')
    ;(chosen ?? items?.[0])?.focus()
  }, [])
  const move = (event: KeyboardEvent) => {
    const step = { ArrowDown: 1, ArrowUp: -1 }[event.key]
    if (!step || !list.current) return
    event.preventDefault()
    const items = [
      ...list.current.querySelectorAll<HTMLButtonElement>(
        '[role="menuitemradio"]:not([aria-disabled="true"]), [role="menuitem"]',
      ),
    ]
    const at = items.indexOf(document.activeElement as HTMLButtonElement)
    items[(at + step + items.length) % items.length]?.focus()
  }
  return (
    <div ref={list} role="menu" aria-label={label} onKeyDown={move} className="flex flex-col">
      {heading && <span className="px-2 pt-1.5 pb-1 text-2xs tracking-[0.06em] text-fg-3 uppercase">{heading}</span>}
      {options.map((option, index) => {
        const checked = option.id === value
        const group = option.group && option.group.key !== options[index - 1]?.group?.key
        return (
          <Fragment key={option.id}>
            {group && (
              <span className="mt-1 flex items-center gap-1.5 border-t border-line-subtle px-2 pt-2 pb-1 text-2xs tracking-[0.06em] text-fg-3 uppercase">
                {option.group?.heading}
              </span>
            )}
            <button
              type="button"
              role="menuitemradio"
              data-id={option.id}
              aria-checked={checked}
              aria-disabled={option.refused ? true : undefined}
              title={option.refused}
              onClick={() => !option.refused && onPick(option.id)}
              className={cx(
                'flex w-full items-center gap-2.5 rounded-sm px-2 py-1.5 text-left text-sm max-md:min-h-11',
                option.refused
                  ? 'cursor-not-allowed text-fg-3'
                  : 'text-fg-1 hover:bg-surface-2 focus:bg-surface-2 focus:outline-none',
                checked && !option.refused && 'bg-surface-2',
              )}
            >
              {option.lead && <span className={cx('flex shrink-0', option.refused && 'opacity-45')}>{option.lead}</span>}
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate">{option.name}</span>
                {option.refused ? (
                  <span className="text-xs text-warn-fg">{option.refused}</span>
                ) : (
                  option.description && <span className="text-xs text-fg-3">{option.description}</span>
                )}
              </span>
              {checked ? (
                <Icon name="check" className="h-3.5 w-3.5 shrink-0 text-accent-fg" />
              ) : (
                <span className="h-3.5 w-3.5 shrink-0" />
              )}
            </button>
          </Fragment>
        )
      })}
      {footer && <div className="mt-1 border-t border-line-subtle px-2 pt-1.5 pb-1 text-2xs text-fg-3">{footer}</div>}
      {action && (
        <div className={cx('mt-1 pt-1', !footer && 'border-t border-line-subtle')}>
          <button
            type="button"
            role="menuitem"
            title={action.title}
            onClick={action.run}
            className="flex w-full items-center gap-2.5 rounded-sm px-2 py-1.5 text-left text-sm text-fg-1 hover:bg-surface-2 focus:bg-surface-2 focus:outline-none max-md:min-h-11"
          >
            {action.lead && <span className="flex shrink-0">{action.lead}</span>}
            <span className="truncate">{action.label}</span>
          </button>
        </div>
      )}
    </div>
  )
}
