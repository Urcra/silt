import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { api, type Queued } from '../api'
import { keys, message } from '../query'
import { Button, IconButton, Input, toast } from '../ui'

function QueuedItem({ thread, item, index, count }: { thread: string; item: Queued; index: number; count: number }) {
  const [editing, setEditing] = useState<string | null>(null)
  const act = (action: Promise<unknown>) => action.catch((failure: unknown) => toast(message(failure), 'danger'))
  if (editing !== null) {
    return (
      <li>
        <form
          className="flex gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            void act(api.editQueued(thread, item.id, editing)).then(() => setEditing(null))
          }}
        >
          <Input aria-label="queued prompt" autoFocus value={editing} onChange={(event) => setEditing(event.target.value)} className="flex-1" />
          <Button type="submit" variant="primary">
            Save
          </Button>
          <Button onClick={() => setEditing(null)}>Cancel</Button>
        </form>
      </li>
    )
  }
  return (
    <li className="flex items-center gap-1 rounded-sm px-1 hover:bg-surface-2">
      <span className="w-5 shrink-0 font-mono text-2xs text-fg-2">{index + 1}</span>
      <button className="min-w-0 flex-1 truncate py-1 text-left" title="edit" onClick={() => setEditing(item.text)}>
        {item.text}
      </button>
      <IconButton
        icon="chevronDown"
        label={`move "${item.text}" up`}
        className="rotate-180"
        disabled={index === 0}
        onClick={() => void act(api.moveQueued(thread, item.id, index - 1))}
      />
      <IconButton
        icon="chevronDown"
        label={`move "${item.text}" down`}
        disabled={index === count - 1}
        onClick={() => void act(api.moveQueued(thread, item.id, index + 1))}
      />
      <Button variant="ghost" size="sm" onClick={() => setEditing(item.text)}>
        Edit
      </Button>
      <Button variant="ghost" size="sm" aria-label={`send "${item.text}" now`} onClick={() => void act(api.sendQueued(thread, item.id))}>
        Send now
      </Button>
      <IconButton icon="x" label={`remove "${item.text}"`} onClick={() => void act(api.removeQueued(thread, item.id))} />
    </li>
  )
}

/** The prompts waiting for the thread's agent (ADR 0022). */
export function PromptQueue({ thread }: { thread: string }) {
  const queue = useQuery({ queryKey: keys.queue(thread), queryFn: () => api.queue(thread) })
  const items = queue.data ?? []
  if (items.length === 0) return null
  return (
    <section aria-label="queued prompts" className="rounded-md border border-line bg-surface-1">
      <div className="flex flex-wrap items-center gap-2 border-b border-line-subtle px-2.5 py-1.5">
        <h4 className="font-mono text-2xs font-medium tracking-[0.04em] text-fg-2 uppercase">Queued · {items.length}</h4>
        <span className="text-2xs text-fg-2">sent in order when the turn ends, kept if anything restarts</span>
      </div>
      <ol className="space-y-0.5 px-1.5 py-1 text-xs">
        {items.map((item, index) => (
          <QueuedItem key={item.id} thread={thread} item={item} index={index} count={items.length} />
        ))}
      </ol>
    </section>
  )
}
