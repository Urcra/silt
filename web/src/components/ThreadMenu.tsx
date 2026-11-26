import { useQuery } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { type ReactNode, useState } from 'react'
import { api, short, type ThreadSummary } from '../api'
import { isChat } from '../chats'
import { keys, message } from '../query'
import { workspaceOf } from '../rules'
import { hosted } from '../served'
import { useThreadState } from '../threadState'
import { Dialog, type MenuItem, toast } from '../ui'
import { ChatToProject } from './ChatToProject'
import { EraseThreadDialog } from './EraseThread'

/**
 * Everything there is to do with a thread, on a right-click in the sidebar or its ⋯: pin, snooze and settle, its id for
 * `silt resume` and `silt fork`, a fork, the history graph, the live view, and for a chat or a Local only thread, moving
 * it to a project or erasing it. `dialogs` are where the last two ask, drawn beside the menu.
 */
export function useThreadMenu(thread: ThreadSummary): { items: MenuItem[]; dialogs: ReactNode } {
  const navigate = useNavigate()
  const state = useThreadState(thread)
  const erasable = useErasable(thread)
  const [asking, setAsking] = useState<'project' | 'erase'>()
  const id = short(thread.id)
  const close = () => setAsking(undefined)
  const items: MenuItem[] = [
    ...state,
    {
      label: 'Copy thread ID',
      hint: id,
      divided: true,
      onSelect: () =>
        void navigator.clipboard?.writeText(id).then(
          () => toast(`Copied ${id}`, 'ok'),
          () => toast('Copying needs the page in focus', 'danger'),
        ),
    },
    {
      label: 'Fork',
      onSelect: () =>
        void api.fork(thread.id).then(
          (made) => navigate({ to: '/t/$id', params: { id: made.thread } }),
          (failure: unknown) => toast(message(failure), 'danger'),
        ),
    },
    ...(isChat(thread.repo) && !hosted() ? [{ label: 'Move to a project…', onSelect: () => setAsking('project') }] : []),
    {
      label: 'History of this thread',
      divided: true,
      onSelect: () => void navigate({ to: '/graph', search: { thread: thread.id } }),
    },
    { label: 'History graph', onSelect: () => void navigate({ to: '/graph' }) },
    ...(hosted()
      ? []
      : [{ label: 'Join live', onSelect: () => void navigate({ to: '/t/$id/live', params: { id: thread.id } }) }]),
    ...(erasable ? [{ label: 'Erase from this machine…', divided: true, danger: true, onSelect: () => setAsking('erase') }] : []),
  ]
  const dialogs = (
    <>
      {asking === 'project' && (
        <Dialog open onClose={close} title="This chat belongs to no project">
          <ChatToProject thread={thread.id} summary={thread} onDone={close} />
        </Dialog>
      )}
      {asking === 'erase' && <EraseThreadDialog thread={thread.id} open onClose={close} />}
    </>
  )
  return { items, dialogs }
}

/** Whether a thread can be erased from this machine: only a Local only one, and never in the hosted app (ADR 0092). */
function useErasable(summary: ThreadSummary): boolean {
  const workspaces = useQuery({ queryKey: keys.workspaces, queryFn: api.workspaces, enabled: !hosted() })
  return !hosted() && Boolean(workspaceOf(workspaces.data, summary.repo, summary.workspace)?.effective.local_only)
}
