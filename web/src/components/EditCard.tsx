import { useState } from 'react'
import type { Change } from '../edits'
import { Icon, cx } from '../ui'

const SHOWN = 12

/** An edit as a small diff in the transcript (ADR 0044); a long one folds to its counts. */
export function EditCard({ change }: { change: Change }) {
  const [all, setAll] = useState(false)
  const rows = [...change.removed.map((text) => ({ sign: '-', text })), ...change.added.map((text) => ({ sign: '+', text }))]
  const shown = all ? rows : rows.slice(0, SHOWN)
  return (
    <div data-testid="edit" className="overflow-hidden rounded-md border border-line bg-elevated">
      <div className="flex items-center gap-2 border-b border-line-subtle px-3 py-1.5 text-xs">
        <Icon name="edit" className="h-3.5 w-3.5 text-fg-2" />
        <span className="text-fg-2">{change.verb}</span>
        <span className="min-w-0 flex-1 truncate font-mono" title={change.file}>
          {change.file}
        </span>
        <span className="shrink-0 font-mono text-2xs">
          <span className="text-ok">+{change.added.length}</span> <span className="text-danger">−{change.removed.length}</span>
        </span>
      </div>
      {rows.length > 0 && (
        <div className="overflow-x-auto py-1 font-mono text-[11.5px] leading-relaxed">
          {shown.map((row, index) => (
            <div key={index} className={cx('px-3 whitespace-pre', row.sign === '-' ? 'bg-danger-tint' : 'bg-ok-tint')}>
              {row.sign} {row.text}
            </div>
          ))}
        </div>
      )}
      {rows.length > SHOWN && (
        <button type="button" onClick={() => setAll(!all)} className="w-full border-t border-line-subtle px-3 py-1 text-left text-2xs text-fg-2 hover:text-fg-1">
          {all ? 'Show fewer lines' : `Show all ${rows.length} lines`}
        </button>
      )}
    </div>
  )
}
