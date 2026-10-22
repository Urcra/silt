import { useQuery } from '@tanstack/react-query'
import { useMatch, useNavigate } from '@tanstack/react-router'
import { type ReactNode, useState } from 'react'
import { api, short } from '../api'
import { useNow } from '../now'
import { projectName } from '../projects'
import { keys, message } from '../query'
import { type Action, formatKeys, useOfferedActions, useShortcut } from '../shortcuts'
import { threadTone } from '../threadLook'
import { ago } from '../time'
import { Dialog, Dot, Icon, type IconName, Kbd, buttonClass, cx, toast } from '../ui'
import { Snippet } from './Snippet'

interface Item {
  id: string
  group: string
  label: ReactNode
  /** What typing matches against. */
  text: string
  icon?: IconName
  dot?: ReactNode
  /** Shown on the right: its keys, or where and when. */
  hint?: ReactNode
  run: () => void
}

/** ⌘K: start work from what's typed, go to a thread or something said, or do an action. */
export function Palette() {
  const [open, setOpen] = useState(false)
  useShortcut('mod+k', 'Open the command palette', () => setOpen((shown) => !shown))
  return (
    <>
      <button
        type="button"
        aria-label="command palette"
        onClick={() => setOpen(true)}
        className={cx(buttonClass('secondary'), 'justify-between text-fg-2 md:min-w-[220px]')}
      >
        <span className="inline-flex items-center gap-1.5">
          <Icon name="search" />
          <span className="hidden md:inline">Go to, run, search</span>
        </span>
        <span className="hidden md:inline-flex">
          <Kbd>{formatKeys('mod+k')}</Kbd>
        </span>
      </button>
      <Dialog open={open} onClose={() => setOpen(false)} title="Command palette" bare>
        {open && <PaletteList close={() => setOpen(false)} />}
      </Dialog>
    </>
  )
}

function PaletteList({ close }: { close: () => void }) {
  const navigate = useNavigate()
  const now = useNow()
  const offered = useOfferedActions()
  const here = useMatch({ from: '/t/$id', shouldThrow: false })?.params.id
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const [query, setQuery] = useState('')
  const [chosen, setChosen] = useState(0)
  const commandsOnly = query.startsWith('>')
  const typed = (commandsOnly ? query.slice(1) : query).trim()
  const wanted = typed.toLowerCase()
  const hits = useQuery({
    queryKey: ['palette-search', wanted],
    queryFn: () => api.search(wanted, {}),
    enabled: !commandsOnly && wanted.length >= 3,
  })
  const current = threads.data?.find((thread) => thread.id === here)
  const project = current ? projectName(current.repo || 'no remote') : undefined
  const goTo = (thread: string, turn?: number | null) => () =>
    void navigate({ to: '/t/$id', params: { id: thread }, hash: turn ? `turn-${turn}` : undefined })
  const failed = (failure: unknown) => toast(message(failure), 'danger')

  const start: Item[] =
    commandsOnly || !typed
      ? []
      : [
          {
            id: 'start:new',
            group: 'Start',
            icon: 'plus',
            text: typed,
            label: (
              <>
                New thread{project && <> in <span className="font-mono">{project}</span></>}: “{typed}”
              </>
            ),
            run: () => void navigate({ to: '/new', search: { prompt: typed } }),
          },
          ...(here
            ? [
                {
                  id: 'start:fork',
                  group: 'Start',
                  icon: 'fork' as const,
                  text: typed,
                  label: 'Fork this thread with that prompt',
                  run: () =>
                    void api
                      .fork(here)
                      .then((made) => api.enqueue(made.thread, typed).then(() => goTo(made.thread)()))
                      .catch(failed),
                },
                {
                  id: 'start:queue',
                  group: 'Start',
                  icon: 'handoff' as const,
                  text: typed,
                  label: 'Queue it in this thread',
                  hint: current?.agent_state === 'working' ? 'turn running' : undefined,
                  run: () =>
                    void api.enqueue(here, typed).then(() => toast('Queued for the agent', 'ok'), failed),
                },
              ]
            : []),
        ]

  const matches = (text: string) => text.toLowerCase().includes(wanted)
  const threadItems: Item[] = commandsOnly
    ? []
    : (threads.data ?? [])
        .filter((thread) => matches(thread.title ?? ''))
        .slice(0, typed ? 6 : 5)
        .map((thread) => ({
          id: `thread:${thread.id}`,
          group: 'Threads',
          text: thread.title ?? '',
          dot: <Dot tone={threadTone(thread)} />,
          label: thread.title ?? `Untitled thread ${short(thread.id)}`,
          hint: (
            <>
              {projectName(thread.repo || 'no remote')} · <span data-volatile>{ago(thread.last_ms, now)}</span>
            </>
          ),
          run: goTo(thread.id),
        }))

  const history: Item[] =
    commandsOnly || wanted.length < 3
      ? []
      : [
          ...(hits.data ?? []).slice(0, 5).map((hit, index) => ({
            id: `hit:${index}`,
            group: 'In history, including before compaction',
            icon: 'clock' as const,
            text: typed,
            label: (
              <>
                “<Snippet text={hit.snippet} />”
                {hit.before_compaction && <span className="ml-2 font-mono text-2xs text-fg-2">before compaction</span>}
              </>
            ),
            hint: (
              <span data-volatile>
                {hit.turn !== null && `turn ${hit.turn} · `}
                {short(hit.thread)}
              </span>
            ),
            run: goTo(hit.thread, hit.turn),
          })),
          {
            id: 'search:all',
            group: 'In history, including before compaction',
            icon: 'search',
            text: typed,
            label: `Search everything for “${typed}”`,
            hint: formatKeys('mod+shift+f'),
            run: () => void navigate({ to: '/search', search: { q: typed } }),
          },
        ]

  const actions: Item[] = offered
    .filter((action: Action) => matches(action.label))
    .map((action) => ({
      id: `action:${action.id}`,
      group: here ? 'This thread and everywhere' : 'Actions',
      text: action.label,
      icon: 'chevronRight',
      label: action.label,
      hint: action.keys ? formatKeys(action.keys) : undefined,
      run: action.run,
    }))

  // An action the words name, or a thread whose title starts with them, is
  // what was meant, so it goes before starting something new with them.
  const named = (item: Item) => wanted !== '' && item.text.toLowerCase().startsWith(wanted)
  const items = [
    ...threadItems.filter(named),
    ...(wanted ? actions : []),
    ...start,
    ...threadItems.filter((item) => !named(item)),
    ...history,
    ...(wanted ? [] : actions),
  ]
  const shown = Math.min(chosen, Math.max(items.length - 1, 0))
  const run = (item: Item | undefined) => {
    if (!item) return
    close()
    item.run()
  }

  return (
    <div className="flex max-h-[min(70vh,640px)] flex-col">
      <input
        aria-label="command"
        autoFocus
        className="w-full border-0 bg-transparent px-4 py-3.5 text-base text-fg-1 outline-none placeholder:text-fg-2 focus-visible:outline-none"
        placeholder="Start something, go to a thread, or search what was said"
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
      <ul role="listbox" aria-label="results" className="min-h-0 flex-1 overflow-y-auto border-t border-line p-1">
        {items.length === 0 && <li className="px-2.5 py-2 text-fg-2">Nothing matches.</li>}
        {items.map((item, index) => (
          <li key={item.id} role="presentation">
            {(index === 0 || items[index - 1].group !== item.group) && (
              <p role="presentation" className="px-2.5 pt-2 pb-1 font-mono text-[10px] tracking-[0.06em] text-fg-2 uppercase">
                {item.group}
              </p>
            )}
            <div
              role="option"
              aria-selected={index === shown}
              onMouseEnter={() => setChosen(index)}
              onClick={() => run(item)}
              className={cx(
                'flex cursor-pointer items-center gap-2.5 rounded-sm px-2.5 py-1.5 text-sm text-fg-1 max-md:min-h-11',
                index === shown && 'bg-surface-2',
              )}
            >
              {item.dot ?? (item.icon && <Icon name={item.icon} className="text-fg-2" />)}
              <span className="min-w-0 flex-1 truncate">{item.label}</span>
              {item.hint && <span className="shrink-0 font-mono text-[10px] text-fg-2">{item.hint}</span>}
            </div>
          </li>
        ))}
      </ul>
      <p className="flex flex-wrap gap-4 border-t border-line px-3.5 py-2 text-2xs text-fg-2">
        <span>
          <Kbd>↑</Kbd> <Kbd>↓</Kbd> move
        </span>
        <span>
          <Kbd>⏎</Kbd> run
        </span>
        <span>
          <Kbd>Esc</Kbd> close
        </span>
        <span className="ml-auto">
          Type <Kbd>&gt;</Kbd> for commands only
        </span>
      </p>
    </div>
  )
}
