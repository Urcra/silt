// The small pieces every view is built of, in Nous.

import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode, SelectHTMLAttributes, TextareaHTMLAttributes } from 'react'
import { type ButtonSize, type ButtonVariant, buttonClass } from './buttonClass'
import { cx } from './cx'
import { Icon, type IconName } from './icons'

export function Button({
  variant = 'secondary',
  size = 'md',
  icon,
  className,
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant; size?: ButtonSize; icon?: IconName }) {
  return (
    <button type="button" className={cx(buttonClass(variant, size), className)} {...props}>
      {icon && <Icon name={icon} />}
      {children}
    </button>
  )
}

/** A button showing only an icon; `label` says what it does. */
export function IconButton({
  icon,
  label,
  className,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { icon: IconName; label: string }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      className={cx(
        'inline-flex items-center justify-center rounded-sm p-1 text-fg-2 hover:bg-surface-2 hover:text-fg-1 max-md:min-h-11 max-md:min-w-11',
        className,
      )}
      {...props}
    >
      <Icon name={icon} />
    </button>
  )
}

const FIELD = 'rounded-sm border border-line bg-elevated px-2 py-1 text-sm text-fg-1 placeholder:text-fg-3 max-md:min-h-11 max-md:text-base'

export function Input({ className, ...props }: InputHTMLAttributes<HTMLInputElement>) {
  return <input className={cx(FIELD, className)} {...props} />
}

export function Textarea({ className, ...props }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea className={cx(FIELD, 'block w-full', className)} {...props} />
}

export function Select({ className, ...props }: SelectHTMLAttributes<HTMLSelectElement>) {
  return <select className={cx(FIELD, className)} {...props} />
}

const TONE = {
  neutral: 'bg-surface-2 text-fg-2',
  accent: 'bg-accent-tint text-accent-fg',
  ok: 'bg-ok-tint text-ok',
  warn: 'bg-warn-tint text-fg-1',
  danger: 'bg-danger-tint text-danger',
  info: 'bg-info-tint text-info',
}

export type Tone = keyof typeof TONE

/** A short label: a status, a kind, a count. */
export function Tag({ tone = 'neutral', children, className }: { tone?: Tone; children: ReactNode; className?: string }) {
  return <span className={cx('inline-flex items-center rounded-sm px-1.5 font-mono text-[11px]', TONE[tone], className)}>{children}</span>
}

/** A tag that can be turned on and off, such as a filter. */
export function Chip({
  selected,
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { selected: boolean }) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      className={cx(
        'rounded-sm border px-2 py-0.5 text-xs',
        selected ? 'border-accent bg-accent-tint text-accent-fg' : 'border-line text-fg-2 hover:bg-surface-2',
      )}
      {...props}
    >
      {children}
    </button>
  )
}

/** A key to press, a hint for the eye only; `onAccent` inside a primary button. */
export function Kbd({ children, onAccent = false }: { children: ReactNode; onAccent?: boolean }) {
  return (
    <kbd
      aria-hidden="true"
      className={cx(
        'inline-flex min-w-[18px] justify-center rounded-sm border border-b-2 px-1 font-mono text-2xs pointer-coarse:hidden',
        onAccent ? 'border-on-accent/30 bg-transparent text-on-accent' : 'border-line bg-surface-1 text-fg-2',
      )}
    >
      {children}
    </kbd>
  )
}

/** A notice across a view; `role` alert for what needs attention now. */
export function Banner({
  tone = 'info',
  children,
  urgent = false,
}: {
  tone?: Exclude<Tone, 'neutral' | 'accent'>
  children: ReactNode
  urgent?: boolean
}) {
  return (
    <div role={urgent ? 'alert' : 'status'} className={cx('flex items-start gap-2 rounded-md px-3 py-2 text-sm', TONE[tone])}>
      <Icon name={tone === 'danger' || tone === 'warn' ? 'alert' : 'info'} className="mt-0.5 shrink-0" />
      <div className="min-w-0">{children}</div>
    </div>
  )
}

export function EmptyState({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="rounded-md border border-dashed border-line px-4 py-8 text-center">
      <p className="text-sm font-medium text-fg-1">{title}</p>
      {children && <div className="mt-1 text-sm text-fg-2">{children}</div>}
    </div>
  )
}

/** A placeholder while content loads. */
export function Skeleton({ className }: { className?: string }) {
  return <span aria-hidden="true" className={cx('block animate-pulse rounded-sm bg-surface-2', className ?? 'h-4 w-full')} />
}

/** Someone's initials in a circle. */
export function Avatar({ name, size = 20 }: { name: string; size?: number }) {
  const initials = name
    .split(/[\s._-]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase())
    .join('')
  return (
    <span
      title={name}
      aria-label={name}
      role="img"
      className="inline-flex shrink-0 items-center justify-center rounded-full bg-[var(--tobacco-300)] font-mono font-semibold text-[var(--paper-50)]"
      style={{ width: size, height: size, fontSize: size * 0.45 }}
    >
      {initials || '?'}
    </span>
  )
}
