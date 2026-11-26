import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { APPROVED } from '../agentState'
import { api, short, type InboxEntry, type InboxRead, type KindSetting, type MrSummary } from '../api'
import { useTopic } from '../bus'
import { KIND_LABEL, notificationsAllowed, subscribeToPush } from '../notifier'
import { useNow } from '../now'
import { useHasServer } from '../hasServer'
import { keys, message } from '../query'
import { ago } from '../time'
import { Button, Dot, type DotTone, EmptyState, Icon, type IconName, Skeleton, Switch, Tabs, buttonClass, toast } from '../ui'
import { ListColumn, Page } from './Page'

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
          <Button
            key={option}
            size="sm"
            variant={index === 0 ? 'primary' : 'secondary'}
            onClick={() => act(api.answer(notification.thread, action.id, option))}
          >
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

/** Each kind of notification under the subject it's about, as Settings groups them; KIND_LABEL names them in full. */
const GROUPS: [string, [kind: string, label: string][]][] = [
  [
    'Agents',
    [
      ['permission', 'Asks for a permission'],
      ['plan', 'Wrote a plan to approve'],
      ['errored', 'Stopped with an error'],
      ['finished', 'Finished its turn'],
      ['offline', 'Was at work on a machine that went offline'],
      ['recovered', 'Made a recovery thread'],
    ],
  ],
  [
    'Merge requests',
    [
      ['mr_opened', 'One into your thread'],
      ['mr_approved', 'Yours was merged'],
      ['mr_declined', 'Yours was declined'],
      ['mr_changes_requested', 'Changes were asked of yours'],
    ],
  ],
  [
    'Conversation',
    [
      ['comment', 'A comment on your thread'],
      ['reply', 'A reply to your comment'],
      ['mention', 'You were mentioned'],
    ],
  ],
]

/** What also puts its thread under Needs you, not only in the inbox. */
const NEEDS_YOU = new Set(['permission', 'plan', 'mr_opened'])

/**
 * Which kinds notify the browser, by subject. Everything lands in the inbox
 * either way, so the inbox column only says so.
 */
export function NotificationSettings() {
  const queries = useQueryClient()
  const settings = useQuery({
    queryKey: keys.notificationSettings,
    queryFn: api.notificationSettings,
    retry: false,
    enabled: useHasServer(),
  })
  if (!settings.data) return null
  const notify = new Map(settings.data.map((setting) => [setting.kind, setting.notify]))
  const grouped = new Set(GROUPS.flatMap(([, kinds]) => kinds.map(([kind]) => kind)))
  // A kind the server knows and this page doesn't yet still gets its switch.
  const others = settings.data
    .filter((setting) => !grouped.has(setting.kind))
    .map((setting): [string, string] => [setting.kind, KIND_LABEL[setting.kind] ?? setting.kind])
  const groups = others.length > 0 ? [...GROUPS, ['Other', others] as (typeof GROUPS)[number]] : GROUPS
  const change = (setting: KindSetting) =>
    void api.setNotification(setting).then(
      (saved) => queries.setQueryData(keys.notificationSettings, saved),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  return (
    <div aria-label="what notifies you" role="group" className="flex flex-col gap-3 text-sm">
      <div className="flex px-3 font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase" aria-hidden="true">
        <span className="flex-1" />
        <span className="w-16 text-center">Inbox</span>
        <span className="w-20 text-center">Browser</span>
      </div>
      {groups.map(([group, kinds]) => {
        const shown = kinds.filter(([kind]) => notify.has(kind))
        if (shown.length === 0) return null
        return (
          <section
            key={group}
            aria-label={group.toLowerCase()}
            className="overflow-hidden rounded-md border border-line bg-surface-1"
          >
            <h3 className="px-3 pt-2.5 pb-1.5 font-medium">{group}</h3>
            <ul>
              {shown.map(([kind, label]) => (
                <li key={kind} className="flex min-h-9 items-center gap-2 border-t border-line-subtle px-3">
                  <span className="min-w-0 flex-1 text-xs">{label}</span>
                  {NEEDS_YOU.has(kind) && (
                    <span className="inline-flex items-center gap-1 rounded-full bg-accent-tint px-2 py-px text-2xs whitespace-nowrap text-accent-fg max-sm:hidden">
                      Also in Needs you
                    </span>
                  )}
                  <span className="flex w-16 justify-center text-fg-3" title="Everything is listed in the inbox">
                    <Icon name="check" className="h-3.5 w-3.5" />
                  </span>
                  <span className="flex w-20 justify-center">
                    <Switch
                      on={notify.get(kind) ?? false}
                      label={KIND_LABEL[kind] ?? label}
                      onChange={(on) => change({ kind, notify: on })}
                    />
                  </span>
                </li>
              ))}
            </ul>
          </section>
        )
      })}
    </div>
  )
}

const BANNER_KEY = 'silt-notifications-asked'

/**
 * Whether this browser shows notifications, and the way to turn them on. In
 * Settings it is the section's status line; elsewhere (`line`) one quiet line,
 * gone once the person says not now.
 */
export function NotificationBanner({ line = false }: { line?: boolean }) {
  const [allowed, setAllowed] = useState(notificationsAllowed())
  const [dismissed, setDismissed] = useState(() => {
    try {
      return localStorage.getItem(BANNER_KEY) === 'no'
    } catch {
      return false
    }
  })
  if (allowed || (line && dismissed) || typeof Notification === 'undefined') return null
  const blocked = Notification.permission === 'denied'
  const turnOn = !blocked && (
    <Button
      variant={line ? 'ghost' : 'secondary'}
      size="sm"
      className={line ? 'text-accent-fg' : undefined}
      onClick={() =>
        void Notification.requestPermission().then(() => {
          setAllowed(notificationsAllowed())
          return subscribeToPush().catch((failure: unknown) => toast(message(failure), 'danger'))
        })
      }
    >
      Turn them on
    </Button>
  )
  if (line)
    return (
      <p role="note" className="flex flex-wrap items-center gap-1.5 text-xs text-fg-2">
        <Icon name="bell" className="h-3.5 w-3.5" />
        {blocked ? 'This browser blocks notifications from this site.' : 'Browser notifications are off.'}
        {turnOn}
        {!blocked && (
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
        )}
      </p>
    )
  return (
    <div role="note" className="flex flex-wrap items-center gap-2 rounded-md bg-warn-tint px-3 py-2 text-xs">
      <Dot tone="warn" />
      <span className="min-w-0 flex-[1_1_320px]">
        {blocked
          ? 'This browser blocks notifications from this site; allow them in its site settings to get these outside the inbox.'
          : 'Browser notifications are off for this site, so only the inbox gets these.'}
      </span>
      {turnOn}
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

/** What brings something to the inbox, for when nothing is there: each leads to where it starts. */
const ARRIVES: [icon: IconName, title: string, detail: string, to: '/needs-you' | '/mrs'][] = [
  ['lock', 'An agent asks to run something', 'Asked before commands, in any of your threads', '/needs-you'],
  ['merge', 'Someone asks you to review', 'A merge request into a thread you own', '/mrs'],
  ['comment', 'An agent or a person asks you', 'Plans to approve, comments, replies and mentions', '/needs-you'],
]

/** Nothing unread: what will arrive, and how to hear of it outside this page. */
function CaughtUp({ waiting }: { waiting: boolean }) {
  return (
    <section aria-label="caught up" className="flex flex-col gap-3 py-2">
      <div className="flex flex-col gap-0.5">
        <h2 className="text-lg font-medium">{waiting ? 'Nothing waits on you' : 'You’re caught up'}</h2>
        <p className="text-fg-2">New items arrive here when:</p>
      </div>
      <ul className="flex flex-col gap-0.5">
        {ARRIVES.map(([icon, title, detail, to]) => (
          <li key={title}>
            <Link
              to={to}
              className="flex items-center gap-3 rounded-sm bg-surface-1 px-3 py-2.5 hover:bg-surface-2 max-md:min-h-11"
            >
              <Icon name={icon} className="h-4 w-4 shrink-0 text-accent-fg" />
              <span className="flex min-w-0 flex-col">
                <span className="text-fg-1">{title}</span>
                <span className="text-xs text-fg-2">{detail}</span>
              </span>
            </Link>
          </li>
        ))}
      </ul>
      <NotificationBanner line />
    </section>
  )
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
    (mrs.data ?? [])
      .filter((mr) => mr.target === thread || mr.fork === thread)
      .sort((a, b) => Number(b.opened_ms) - Number(a.opened_ms))[0]
  const notifications = inbox.data ?? []
  const unread = notifications.filter((notification) => !notification.read)
  const waiting = unread.filter((notification) => notification.action !== null)
  const shown = filter === 'unread' ? unread : filter === 'waiting' ? waiting : notifications
  // An empty inbox still shows what was read lately, so it isn't a blank page.
  const recent = filter === 'all' || shown.length > 0 ? [] : notifications.filter((notification) => notification.read).slice(0, 5)
  const markRead = async (read: Partial<InboxRead>, server?: string | null) => {
    await api.readNotifications(read, server)
    await queries.invalidateQueries({ queryKey: ['inbox'] })
  }
  const days = (['Today', 'Earlier'] as const).map(
    (day) => [day, shown.filter((notification) => dayOf(Number(notification.created_ms), now) === day)] as const,
  )
  const row = (notification: InboxEntry) => (
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
  )
  return (
    <Page
      flush
      label="inbox"
      title="Inbox"
      meta={`${unread.length} unread`}
      actions={
        <>
          {unread.length > 0 && (
            <Button variant="ghost" icon="check" onClick={() => void markRead({ all: true })}>
              Mark all read
            </Button>
          )}
          <Link to="/settings/$section" params={{ section: 'notifications' }} className={buttonClass('ghost')}>
            <Icon name="bell" />
            What notifies you
          </Link>
        </>
      }
    >
      <div className="flex w-full max-w-[960px] flex-wrap items-center gap-2 px-4 pt-4 md:pr-6 md:pl-5">
        <Tabs
          label="inbox filter"
          value={filter}
          onChange={setFilter}
          tabs={[
            {
              id: 'unread',
              label: (
                <>
                  Unread <span className="font-mono text-fg-2">{unread.length}</span>
                </>
              ),
            },
            {
              id: 'waiting',
              label: (
                <>
                  Waiting on me <span className="font-mono text-fg-2">{waiting.length}</span>
                </>
              ),
            },
            {
              id: 'all',
              label: (
                <>
                  All <span className="font-mono text-fg-2">{notifications.length}</span>
                </>
              ),
            },
          ]}
        />
      </div>
      <ListColumn>
        {inbox.error && <p className="text-danger">{message(inbox.error)}</p>}
        {inbox.isPending && <Skeleton className="h-24" />}
        {inbox.isSuccess &&
          shown.length === 0 &&
          (filter === 'all' ? <EmptyState title="Nothing yet" /> : <CaughtUp waiting={filter === 'waiting'} />)}
        {inbox.isSuccess && shown.length > 0 && <NotificationBanner line />}
        <div data-testid="inbox" className="flex flex-col gap-4">
          {days.map(
            ([day, items]) =>
              items.length > 0 && (
                <section key={day} aria-label={day.toLowerCase()} className="flex flex-col gap-1.5">
                  <h2 className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">{day}</h2>
                  <ul className="overflow-hidden rounded-md border border-line bg-elevated">{items.map(row)}</ul>
                </section>
              ),
          )}
        </div>
        {recent.length > 0 && (
          <section aria-label="read lately" className="flex flex-col gap-1.5 pt-2">
            <h2 className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">Read lately</h2>
            <ul className="overflow-hidden rounded-md border border-line-subtle">{recent.map(row)}</ul>
          </section>
        )}
      </ListColumn>
    </Page>
  )
}
