import type { KeyboardEvent, ReactNode } from 'react'
import { Icon, type IconName, cx } from '../ui'

/**
 * The one composer (ADR 0044): centred on an empty thread, docked under a
 * running one. What's above the text, the text, then a row of its tools and
 * the button that sends it.
 */
export function ComposerBox({
  id,
  label,
  value,
  onChange,
  onSend,
  placeholder,
  rows = 3,
  above,
  below,
  tools,
  actions,
  waiting = false,
  onCycle,
  className,
}: {
  id: string
  /** What the text is, for a screen reader and the tests. */
  label: string
  value: string
  onChange: (value: string) => void
  /** mod+⏎, with whether shift was held too. */
  onSend: (shift: boolean) => void
  placeholder: string
  rows?: number
  /** Over the text: what's queued, what others are writing. */
  above?: ReactNode
  /** Between the text and the tools: attached files. */
  below?: ReactNode
  tools?: ReactNode
  actions?: ReactNode
  /** The agent waits on the person, so the box says so. */
  waiting?: boolean
  /** Shift+Tab: the agent's next mode, as in its own terminal. */
  onCycle?: () => void
  className?: string
}) {
  const keyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
      event.preventDefault()
      onSend(event.shiftKey)
    }
    if (event.key === 'Tab' && event.shiftKey && onCycle) {
      event.preventDefault()
      onCycle()
    }
  }
  return (
    <div className={cx('flex flex-col rounded-lg border bg-elevated', waiting ? 'border-warn' : 'border-line-strong', className)}>
      {above}
      <label htmlFor={id} className="sr-only">
        {label}
      </label>
      <textarea
        id={id}
        aria-label={label}
        rows={rows}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={keyDown}
        placeholder={placeholder}
        className="resize-none border-0 bg-transparent px-4 pt-3.5 pb-1.5 text-sm leading-normal text-fg-1 placeholder:text-fg-3 focus:outline-none max-md:text-base"
      />
      {below}
      <div className="flex items-center gap-1 px-2.5 pt-1 pb-2.5">
        <span className="flex min-w-0 flex-1 flex-wrap items-center gap-1 *:shrink-0">{tools}</span>
        <span className="flex shrink-0 items-center gap-2">{actions}</span>
      </div>
    </div>
  )
}

/** The composer's round button: send, start or stop. */
export function RoundButton({
  label,
  icon,
  onClick,
  disabled,
  type = 'button',
  quiet = false,
  title,
}: {
  label: string
  icon: IconName
  onClick?: () => void
  disabled?: boolean
  type?: 'button' | 'submit'
  /** Stop: drawn on the surface rather than in amber. */
  quiet?: boolean
  title?: string
}) {
  return (
    <button
      type={type}
      aria-label={label}
      title={title ?? label}
      onClick={onClick}
      disabled={disabled}
      className={cx(
        'inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full max-md:h-11 max-md:w-11',
        quiet
          ? 'border border-line-strong bg-surface-2 text-fg-1 hover:bg-surface-3'
          : 'bg-accent text-on-accent hover:bg-accent-hover disabled:bg-surface-2 disabled:text-fg-3',
      )}
    >
      <Icon name={icon} className="h-4 w-4" />
    </button>
  )
}
