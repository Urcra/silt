import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { APPROVED } from '../agentState'
import { api, short, type InboxEntry, type InboxRead, type KindSetting, type MrSummary } from '../api'
import { useTopic } from '../bus'
import { KIND_LABEL, notificationsAllowed, subscribeToPush } from '../notifier'
import { useNow } from '../now'
import { keys, message } from '../query'
import { ago } from '../time'
import { Button, Dot, type DotTone, EmptyState, Skeleton, Tabs, buttonClass, toast } from '../ui'
import { Page } from './Page'

const TONE: Record<string, DotTone> = {
  permission: 'warn',
  plan: 'warn',
  errored: 'danger',
  offline: 'danger',
  recovered: 'danger',
  finished: 'ok',
  mr_opened: 'accent',
  mr_approved: 'ok',
  mr_declined: 'idle',
  mr_changes_requested: 'warn',
  comment: 'info',
  reply: 'info',
  mention: 'info',
}

/** What the notification lets the person do in place: answer the agent, then it's read (ADR 0028). */
function Answer({ notification, done }: { notification: InboxEntry; done: () => Promise<void> }) {
  const action = notification.action
  if (!action || notification.read) return null
  const act = (call: Promise<unknown>) => void call.then(done, (failure: unknown) => toast(message(failure), 'danger'))
  return (
    <span className="flex flex-wrap gap-1" role="group" aria-label={`answer ${notification.id}`}>
      {action.do === 'permission' &&
        action.options.map(([option, label], index) => (
          <Button key={option} size="sm" variant={index === 0 ? 'primary' : 'secondary'} onClick={() => act(api.answer(notification.thread, action.id, option))}>
            {label}
          </Button>
        ))}
      {action.do === 'plan' && (
        <Button variant="primary" size="sm" onClick={() => act(api.prompt(notification.thread, APPROVED))}>
          Approve plan
        </Button>
      )}
    </span>
  )
}

/** Where a notification leads, as its button says. */
function Go({ notification, review }: { notification: InboxEntry; review: MrSummary | undefined }) {
  const button = buttonClass('secondary', 'sm')
  if (notification.kind === 'recovered')
    return (
      <Link to="/t/$id/reconcile" params={{ id: notification.thread }} className={button}>
        Review
      </Link>
    )
  if (notification.kind.startsWith('mr_') && review)
    return (
      <Link to="/mr/$id" params={{ id: review.id }} className={button}>
        Review
      </Link>
    )
  return (
    <Link to="/t/$id" params={{ id: notification.thread }} className={buttonClass('ghost', 'sm')}>
      {['comment', 'reply', 'mention'].includes(notification.kind) ? 'Reply' : 'Open'}
    </Link>
  )
}

/** Which kinds notify, as a quiet list under the inbox. */
function NotificationSettings() {
  const queries = useQueryClient()
  const settings = useQuery({ queryKey: keys.notificationSettings, queryFn: api.notificationSettings, retry: false })
  if (!settings.data) return null
  const change = (setting: KindSetting) =>
    void api.setNotification(setting).then(
      (saved) => queries.setQueryData(keys.notificationSettings, saved),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  return (
    <details className="rounded-md border border-line px-3 py-2 text-sm">
      <summary className="flex cursor-pointer items-center text-xs text-fg-2 max-md:min-h-11">What notifies you</summary>
      <fieldset className="mt-2 grid gap-1 pb-1 sm:grid-cols-2">
        <legend className="mb-1 text-xs text-fg-2">Notify me, not only list it, when:</legend>
        {settings.data.map((setting) => (
          <label key={setting.kind} className="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              checked={setting.notify}
              onChange={(event) => change({ kind: setting.kind, notify: event.target.checked })}
              className="accent-[var(--accent)]"
            />
            {KIND_LABEL[setting.kind] ?? setting.kind}
          </label>
        ))}
      </fieldset>
    </details>
  )
}

const BANNER_KEY = 'silt-notifications-asked'

/** Asks once whether to show browser notifications, until the person says yes or not now. */
function NotificationBanner() {
  const [allowed, setAllowed] = useState(notificationsAllowed())
  const [dismissed, setDismissed] = useState(() => {
    try {
      return localStorage.getItem(BANNER_KEY) === 'no'
    } catch {
      return false
    }
  })
  if (allowed || dismissed || typeof Notification === 'undefined') return null
  return (
    <div role="note" className="flex flex-wrap items-center gap-2 rounded-md border border-line bg-surface-1 px-3 py-2.5 text-xs">
      <span className="min-w-0 flex-[1_1_320px]">Get a browser notification when an agent asks for permission or someone asks you for a review.</span>
      <Button
        size="sm"
        onClick={() =>
          void Notification.requestPermission().then(() => {
            setAllowed(notificationsAllowed())
            return subscribeToPush().catch((failure: unknown) => toast(message(failure), 'danger'))
          })
        }
      >
        Turn on notifications
      </Button>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => {
          setDismissed(true)
          try {
            localStorage.setItem(BANNER_KEY, 'no')
          } catch {
            // Asked again next time, then.
          }
        }}
      >
        Not now
      </Button>
    </div>
  )
}

type Filter = 'unread' | 'waiting' | 'all'

/** The day a notification belongs to, as the inbox groups them. */
function dayOf(ms: number, now: number): 'Today' | 'Earlier' {
  const start = new Date(now)
  start.setHours(0, 0, 0, 0)
  return ms >= start.getTime() ? 'Today' : 'Earlier'
}

/** What waits on the person, and what happened in their threads (the Inbox board). */
export function Inbox() {
  useTopic('inbox')
  const now = useNow()
  const [filter, setFilter] = useState<Filter>('unread')
  const queries = useQueryClient()
  const inbox = useQuery({ queryKey: keys.inbox(true), queryFn: () => api.inbox(true) })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const mrs = useQuery({ queryKey: keys.mrs, queryFn: api.mergeRequests })
  const titleOf = (thread: string) => threads.data?.find((summary) => summary.id === thread)?.title ?? 'Untitled thread'
  const reviewOf = (thread: string) =>
    (mrs.data ?? []).filter((mr) => mr.target === thread || mr.fork === thread).sort((a, b) => Number(b.opened_ms) - Number(a.opened_ms))[0]
  const notifications = inbox.data ?? []
  const unread = notifications.filter((notification) => !notification.read)
  const waiting = unread.filter((notification) => notification.action !== null)
  const shown = filter === 'unread' ? unread : filter === 'waiting' ? waiting : notifications
  const markRead = async (read: Partial<InboxRead>, server?: string | null) => {
    await api.readNotifications(read, server)
    await queries.invalidateQueries({ queryKey: ['inbox'] })
  }
  const days = (['Today', 'Earlier'] as const).map((day) => [day, shown.filter((notification) => dayOf(Number(notification.created_ms), now) === day)] as const)
  return (
    <Page flush label="inbox">
      <div className="flex min-h-10 flex-wrap items-center gap-2 border-b border-line px-4 py-1.5 md:px-8">
        <Tabs
          label="inbox filter"
          value={filter}
          onChange={setFilter}
          tabs={[
            { id: 'unread', label: <>Unread <span className="font-mono text-fg-2">{unread.length}</span></> },
            { id: 'waiting', label: <>Waiting on me <span className="font-mono text-fg-2">{waiting.length}</span></> },
            { id: 'all', label: 'All' },
          ]}
        />
        <Button variant="ghost" icon="check" className="ml-auto" disabled={unread.length === 0} onClick={() => void markRead({ all: true })}>
          Mark all read
        </Button>
      </div>
      <div className="flex max-w-[960px] flex-col gap-4 px-4 py-4 md:px-8">
        <NotificationBanner />
        {inbox.error && <p className="text-danger">{message(inbox.error)}</p>}
        {inbox.isPending && <Skeleton className="h-24" />}
        {inbox.isSuccess && shown.length === 0 && (
          <EmptyState title={filter === 'waiting' ? 'Nothing waits on you' : 'Nothing new'}>
            Agents asking for permission, reviews and replies show here.
          </EmptyState>
        )}
        <div data-testid="inbox" className="flex flex-col gap-4">
          {days.map(
            ([day, items]) =>
              items.length > 0 && (
                <section key={day} aria-label={day.toLowerCase()} className="flex flex-col gap-1.5">
                  <h2 className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">{day}</h2>
                  <ul className="overflow-hidden rounded-md border border-line bg-elevated">
                    {items.map((notification) => (
                      <li
                        key={`${notification.server ?? ''}:${notification.id}`}
                        className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line-subtle px-3 py-2.5 last:border-b-0"
                      >
                        <Dot tone={notification.read ? 'idle' : (TONE[notification.kind] ?? 'info')} />
                        <div className="flex min-w-0 flex-[1_1_240px] flex-col gap-0.5">
                          <span className={notification.read ? 'text-fg-2' : 'text-fg-1'}>{notification.text}</span>
                          <span className="font-mono text-2xs text-fg-2">
                            <Link to="/t/$id" params={{ id: notification.thread }} className="text-accent-fg hover:text-fg-1">
                              {titleOf(notification.thread)}
                            </Link>
                            {' · '}
                            <span data-volatile>{short(notification.thread)}</span>
                            {notification.actor && ` · ${notification.actor}`}
                            {' · '}
                            <span data-volatile>{ago(Number(notification.created_ms), now)}</span>
                          </span>
                        </div>
                        <span className="flex items-center gap-1 max-sm:w-full max-sm:pl-[18px]">
                          <Answer notification={notification} done={() => markRead({ ids: [notification.id] }, notification.server)} />
                          {!notification.action && <Go notification={notification} review={reviewOf(notification.thread)} />}
                          {!notification.read && (
                            <Button
                              variant="ghost"
                              size="sm"
                              aria-label={`mark ${notification.id} read`}
                              title="Mark read"
                              onClick={() => void markRead({ ids: [notification.id] }, notification.server)}
                            >
                              Read
                            </Button>
                          )}
                        </span>
                      </li>
                    ))}
                  </ul>
                </section>
              ),
          )}
        </div>
        <NotificationSettings />
      </div>
    </Page>
  )
}
