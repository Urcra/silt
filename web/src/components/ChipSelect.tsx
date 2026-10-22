import { Icon, type IconName, cx } from '../ui'

export const CHIP =
  'inline-flex items-center gap-1.5 rounded-sm border border-line bg-elevated px-2 py-0.5 text-xs text-fg-1 hover:bg-surface-2 max-md:min-h-11'

/** A choice in a composer's row, drawn as a chip: `prefix`, then the chosen option. */
export function ChipSelect({
  label,
  prefix,
  icon,
  value,
  onChange,
  options,
  mono = false,
}: {
  label: string
  prefix?: string
  icon?: IconName
  value: string
  onChange: (value: string) => void
  options: { id: string; name: string; description?: string | null }[]
  mono?: boolean
}) {
  return (
    <label className={cx(CHIP, 'relative cursor-pointer pr-1')}>
      {icon && <Icon name={icon} className="h-3 w-3" />}
      {prefix && <span className="text-fg-2">{prefix}</span>}
      <select
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        className={cx('max-w-56 cursor-pointer appearance-none truncate bg-transparent pr-4 text-xs text-fg-1 focus:outline-none', mono && 'font-mono')}
      >
        {options.map((option) => (
          <option key={option.id} value={option.id} title={option.description ?? undefined}>
            {option.name}
          </option>
        ))}
      </select>
      <Icon name="chevronDown" className="pointer-events-none absolute right-1 h-3 w-3 text-fg-2" />
    </label>
  )
}
