import { Icon, type IconName, Spinner, cx } from '../ui'

/** The composer's round button: send, start, stop or dictate. */
export function RoundButton({
  label,
  icon,
  onClick,
  disabled,
  type = 'button',
  quiet = false,
  busy = false,
  title,
}: {
  label: string
  icon: IconName
  onClick?: () => void
  disabled?: boolean
  type?: 'button' | 'submit'
  /** Stop: drawn on the surface rather than in amber. */
  quiet?: boolean
  /** What it did is under way: a spinner in place of its icon. */
  busy?: boolean
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
      {busy ? <Spinner className="h-4 w-4" /> : <Icon name={icon} className="h-4 w-4" />}
    </button>
  )
}
