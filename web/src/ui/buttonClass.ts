import { cx } from './cx'

const BUTTON = {
  primary: 'border-transparent bg-accent text-on-accent hover:bg-accent-hover',
  secondary: 'border-line bg-surface-1 text-fg-1 hover:bg-surface-2',
  ghost: 'border-transparent text-fg-2 hover:bg-surface-2 hover:text-fg-1',
  danger: 'border-line bg-surface-1 text-danger hover:bg-danger-tint',
}

export type ButtonVariant = keyof typeof BUTTON

const SIZE = { md: 'px-2.5 py-1', sm: 'px-1.5 py-0.5', lg: 'px-4 py-2.5 text-sm' }

export type ButtonSize = keyof typeof SIZE

/** A button's look, also for links that act as buttons; at least 44 px tall on a phone. */
export function buttonClass(variant: ButtonVariant = 'secondary', size: ButtonSize = 'md'): string {
  return cx(
    'inline-flex shrink-0 items-center gap-1.5 rounded-sm border text-xs font-medium whitespace-nowrap transition-colors duration-100 disabled:cursor-not-allowed disabled:opacity-40 max-md:min-h-11',
    SIZE[size],
    BUTTON[variant],
  )
}
