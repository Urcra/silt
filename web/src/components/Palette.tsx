import { useQuery } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { api, short } from '../api'
import { keys } from '../query'
import { type Action, formatKeys, useOfferedActions, useShortcut } from '../shortcuts'
import { Dialog, Input, Kbd, cx } from '../ui'

interface Item {
  id: string
  group: string
  label: string
  keys?: string
  run: () => void
}

/** ⌘K: every action on offer, the threads, and what history says, by typing. */
export function Palette() {
  const [open, setOpen] = useState(false)
  useShortcut('mod+k', 'Open the command palette', () => setOpen((shown) => !shown))
  return (
    <>
      <button
        type="button"
        aria-label="command palette"
        onClick={() => setOpen(true)}
        className="flex items-center gap-2 rounded-sm border border-line px-2 py-0.5 text-fg-2 hover:bg-surface-2 hover:text-fg-1"
      >
        Go to or do <Kbd>{formatKeys('mod+k')}</Kbd>
      </button>
      <Dialog open={open} onClose={() => setOpen(false)} title="Go to or do">
        {open && <PaletteList close={() => setOpen(false)} />}
      </Dialog>
    </>
  )
}

function PaletteList({ close }: { close: () => void }) {
  const navigate = useNavigate()
  const offered = useOfferedActions()
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const [query, setQuery] = useState('')
  const [chosen, setChosen] = useState(0)
  const wanted = query.trim().toLowerCase()
  const hits = useQuery({
    queryKey: ['palette-search', wanted],
    queryFn: () => api.search(wanted, {}),
    enabled: wanted.length >= 3,
  })
  const goTo = (thread: string) => () => void navigate({ to: '/t/$id', params: { id: thread } })
  const items: Item[] = [
    ...offered.map((action: Action) => ({ ...action, group: 'Actions' })),
    ...(threads.data ?? []).map((thread) => ({
      id: `thread:${thread.id}`,
      group: 'Threads',
      label: thread.title ?? `Untitled thread ${short(thread.id)}`,
      run: goTo(thread.id),
    })),
  ].filter((item) => item.label.toLowerCase().includes(wanted))
  for (const [index, hit] of (hits.data ?? []).slice(0, 8).entries()) {
    items.push({ id: `hit:${index}`, group: 'History', label: hit.snippet, run: goTo(hit.thread) })
  }
  const shown = Math.min(chosen, Math.max(items.length - 1, 0))
  const run = (item: Item | undefined) => {
    if (!item) return
    close()
    item.run()
  }

  return (
    <div>
      <Input
        aria-label="command"
        autoFocus
        className="w-full"
        placeholder="Type an action, a thread, or something said"
        value={query}
        onChange={(event) => {
          setQuery(event.target.value)
          setChosen(0)
        }}
        onKeyDown={(event) => {
          if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault()
            const step = event.key === 'ArrowDown' ? 1 : -1
            setChosen((shown + step + items.length) % Math.max(items.length, 1))
          }
          if (event.key === 'Enter') {
            event.preventDefault()
            run(items[shown])
          }
        }}
      />
      <ul role="listbox" aria-label="results" className="mt-2 max-h-80 overflow-y-auto text-sm">
        {items.length === 0 && <li className="px-2 py-1 text-fg-3">Nothing matches.</li>}
        {items.map((item, index) => (
          <li
            key={item.id}
            role="option"
            aria-selected={index === shown}
            onMouseEnter={() => setChosen(index)}
            onClick={() => run(item)}
            className={cx('flex cursor-pointer items-center gap-2 rounded-sm px-2 py-1', index === shown && 'bg-accent-tint')}
          >
            {(index === 0 || items[index - 1].group !== item.group) && (
              <span className="w-16 shrink-0 font-mono text-[10px] tracking-wider text-fg-3 uppercase">{item.group}</span>
            )}
            {index > 0 && items[index - 1].group === item.group && <span className="w-16 shrink-0" />}
            <span className="min-w-0 flex-1 truncate">{item.label}</span>
            {item.keys && <Kbd>{formatKeys(item.keys)}</Kbd>}
          </li>
        ))}
      </ul>
    </div>
  )
}
