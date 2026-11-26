import type { MachineMarked } from '../elsewhere'
import { cx } from '../ui'

/** The machine's name in a chip, plain or in its colour, before a thread's state. */
export function MachineChip({ marked }: { marked: MachineMarked }) {
  const coloured = marked.mark === 'colour-chip'
  return (
    <span
      data-testid="machine-chip"
      className={cx(
        'inline-flex h-[15px] shrink-0 items-center rounded-[3px] px-[5px] font-mono text-mini leading-none',
        coloured ? 'font-medium text-elevated' : 'border border-line bg-surface-1 text-fg-2',
      )}
      style={coloured ? { background: marked.colour } : undefined}
    >
      {marked.name}
    </span>
  )
}

/** The machine's colour as a stripe at the row's edge. */
export function MachineStripe({ marked }: { marked: MachineMarked }) {
  return (
    <span
      aria-hidden="true"
      data-testid="machine-stripe"
      className="pointer-events-none absolute top-[7px] bottom-[7px] left-[9px] w-[3px] rounded-sm"
      style={{ background: marked.colour }}
    />
  )
}

/** The machine's initial on its colour, at the row's end. */
export function MachineInitial({ marked }: { marked: MachineMarked }) {
  return (
    <span
      aria-hidden="true"
      data-testid="machine-initial"
      className="mt-0.5 inline-flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-[3px] font-mono text-mini font-semibold text-elevated"
      style={{ background: marked.colour }}
    >
      {[...marked.name][0]?.toUpperCase()}
    </span>
  )
}
