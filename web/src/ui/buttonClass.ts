import { cx } from './cx'

// A disabled button keeps a readable fg-2 label on a flat fill, rather than
// fading, which drops below 4.5:1 on Nous's warm surfaces.
const BUTTON = {
  primary: 'border-transparent bg-accent text-on-accent hover:bg-accent-hover disabled:bg-surface-2 disabled:text-fg-2',
  secondary: 'border-line bg-surface-1 text-fg-1 hover:bg-surface-2 disabled:bg-surface-1 disabled:text-fg-2',
  ghost: 'border-transparent text-fg-2 hover:bg-surface-2 hover:text-fg-1 disabled:bg-transparent disabled:text-fg-2',
  danger: 'border-line bg-surface-1 text-danger hover:bg-danger-tint disabled:bg-surface-1 disabled:text-fg-2',
}

export type ButtonVariant = keyof typeof BUTTON

const SIZE = {
  md: 'px-2.5 py-1 text-xs',
  sm: 'px-1.5 py-0.5 text-xs',
  lg: 'px-4 py-2.5 text-sm',
  touch: 'min-h-11 justify-center px-3 text-sm',
}

export type ButtonSize = keyof typeof SIZE

/** A button's look, also for links that act as buttons; at least 44 px tall on a phone. */
export function buttonClass(variant: ButtonVariant = 'secondary', size: ButtonSize = 'md'): string {
  return cx(
    'inline-flex shrink-0 items-center gap-1.5 rounded-sm border font-medium whitespace-nowrap transition-colors duration-100 disabled:cursor-not-allowed max-md:min-h-11',
    SIZE[size],
    BUTTON[variant],
  )
}
