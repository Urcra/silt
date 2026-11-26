import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { api, short, type StateChange, type ThreadSummary } from './api'
import { useNow } from './now'
import { keys, message } from './query'
import { snoozed, tomorrowMorning } from './sections'
import { hosted } from './served'
import { type MenuItem, toast } from './ui'

const HOUR_MS = 60 * 60 * 1000
/** Far enough ahead that only activity ends it. */
const UNTIL_ACTIVITY_MS = 365 * 24 * HOUR_MS
const NOTHING: StateChange = { pinned: null, snooze: null, settled: null, seen: false, reviewed_turn: null }

/** Pin, snooze or settle a thread for this person (ADR 0024), as menu items. */
export function useThreadState(thread: ThreadSummary): { label: string; onSelect: () => void }[] {
  const queries = useQueryClient()
  const change = (what: Partial<StateChange>, said: string) => () =>
    void api.setThreadState(thread.id, { ...NOTHING, ...what }).then(
      () => {
        toast(said)
        return queries.invalidateQueries({ queryKey: keys.threads })
      },
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const now = useNow()
  // Until when is worked out at the click, not when the menu was drawn.
  const snooze = (until: (now: number) => number, said: string) => () =>
    change({ snooze: { until: until(Date.now()), wake_on_activity: true } }, said)()
  return [
    thread.mine.pinned
      ? { label: 'Unpin', onSelect: change({ pinned: false }, 'Unpinned') }
      : { label: 'Pin', onSelect: change({ pinned: true }, 'Pinned') },
    ...(snoozed(thread, now)
      ? [{ label: 'Wake up', onSelect: change({ snooze: { until: null, wake_on_activity: false } }, 'Woken') }]
      : [
          { label: 'Snooze for an hour', onSelect: snooze((at) => at + HOUR_MS, 'Snoozed for an hour') },
          { label: 'Snooze until tomorrow', onSelect: snooze((at) => tomorrowMorning(new Date(at)), 'Snoozed until tomorrow') },
          {
            label: 'Snooze until something happens',
            onSelect: snooze((at) => at + UNTIL_ACTIVITY_MS, 'Snoozed until something happens'),
          },
        ]),
    thread.mine.settled
      ? { label: 'Unsettle', onSelect: change({ settled: false }, 'Back among the active') }
      : { label: 'Settle', onSelect: change({ settled: true }, 'Settled') },
  ]
}

/**
 * Everything the sidebar offers for a thread, on a right-click or its ⋯: pin, snooze and settle, then its id for `silt
 * resume` and `silt fork`, a fork, the history graph and the live view.
 */
export function useThreadMenu(thread: ThreadSummary): MenuItem[] {
  const navigate = useNavigate()
  const state = useThreadState(thread)
  const id = short(thread.id)
  return [
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
    { label: 'History graph', divided: true, onSelect: () => void navigate({ to: '/graph' }) },
    ...(hosted()
      ? []
      : [{ label: 'Join live', onSelect: () => void navigate({ to: '/t/$id/live', params: { id: thread.id } }) }]),
  ]
}
