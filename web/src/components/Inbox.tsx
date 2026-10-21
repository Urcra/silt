import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { APPROVED } from '../agentState'
import { api, short, type InboxEntry, type InboxRead, type KindSetting } from '../api'
import { useTopic } from '../bus'
import { KIND_LABEL, notificationsAllowed, subscribeToPush } from '../notifier'
import { keys, message } from '../query'
import { Button, toast } from '../ui'

/** What the notification lets the person do in place: answer the agent, then it's read (ADR 0028). */
function Actions({ notification, done }: { notification: InboxEntry; done: () => Promise<void> }) {
  const action = notification.action
  if (!action || notification.read) return null
  const act = (call: Promise<unknown>) =>
    void call.then(done, (failure: unknown) => toast(message(failure), 'danger'))
  return (
    <span className="ml-2 inline-flex gap-1" role="group" aria-label={`answer ${notification.id}`}>
      {action.do === 'permission' &&
        action.options.map(([option, label]) => (
          <Button key={option} className="px-1.5 py-0.5 text-xs" onClick={() => act(api.answer(notification.thread, action.id, option))}>
            {label}
          </Button>
        ))}
      {action.do === 'plan' && (
        <Button variant="primary" className="px-1.5 py-0.5 text-xs" onClick={() => act(api.prompt(notification.thread, APPROVED))}>
          Approve plan
        </Button>
      )}
    </span>
  )
}

/** Which kinds notify, and asking the browser to show notifications at all. */
function NotificationSettings() {
  const queries = useQueryClient()
  const settings = useQuery({ queryKey: keys.notificationSettings, queryFn: api.notificationSettings, retry: false })
  const [allowed, setAllowed] = useState(notificationsAllowed())
  if (!settings.data) return null
  const change = (setting: KindSetting) =>
    void api.setNotification(setting).then(
      (saved) => queries.setQueryData(keys.notificationSettings, saved),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  return (
    <details className="mb-4 rounded border border-line p-3 text-sm">
      <summary className="cursor-pointer">Notifications</summary>
      {!allowed && typeof Notification !== 'undefined' && (
        <Button
          className="mt-2"
          onClick={() =>
            void Notification.requestPermission().then(() => {
              setAllowed(notificationsAllowed())
              return subscribeToPush().catch((failure: unknown) => toast(message(failure), 'danger'))
            })
          }
        >
          Allow browser notifications
        </Button>
      )}
      <fieldset className="mt-2 grid gap-1 sm:grid-cols-2">
        <legend className="mb-1 text-xs text-fg-2">Notify me, not only list it, when:</legend>
        {settings.data.map((setting) => (
          <label key={setting.kind} className="flex items-center gap-2">
            <input type="checkbox" checked={setting.notify} onChange={(event) => change({ kind: setting.kind, notify: event.target.checked })} />
            {KIND_LABEL[setting.kind] ?? setting.kind}
          </label>
        ))}
      </fieldset>
    </details>
  )
}

export function Inbox() {
  useTopic('inbox')
  const [all, setAll] = useState(false)
  const queries = useQueryClient()
  const inbox = useQuery({ queryKey: keys.inbox(all), queryFn: () => api.inbox(all) })
  const notifications = inbox.data ?? []
  const markRead = async (read: Partial<InboxRead>, server?: string | null) => {
    await api.readNotifications(read, server)
    await queries.invalidateQueries({ queryKey: ['inbox'] })
  }
  return (
    <div>
      <div className="mb-3 flex items-center gap-4">
        <h2 className="text-lg font-semibold">Inbox</h2>
        <label className="text-sm">
          <input type="checkbox" checked={all} onChange={(event) => setAll(event.target.checked)} /> show read
        </label>
        <button className="text-sm text-accent-fg" onClick={() => void markRead({ all: true })}>
          Mark all read
        </button>
      </div>
      <NotificationSettings />
      {inbox.error && <p className="text-danger">{message(inbox.error)}</p>}
      {inbox.isSuccess && notifications.length === 0 && <p className="text-fg-3">Nothing new.</p>}
      <ul data-testid="inbox" className="space-y-2">
        {notifications.map((notification) => (
          <li key={`${notification.server ?? ''}:${notification.id}`} className={notification.read ? 'text-fg-3' : ''}>
            <Link className="text-accent-fg" to="/t/$id" params={{ id: notification.thread }}>
              {short(notification.thread)}
            </Link>{' '}
            {notification.text}
            <Actions notification={notification} done={() => markRead({ ids: [notification.id] }, notification.server)} />{' '}
            {!notification.read && (
              <button
                className="text-xs text-accent-fg"
                aria-label={`mark ${notification.id} read`}
                onClick={() => void markRead({ ids: [notification.id] }, notification.server)}
              >
                mark read
              </button>
            )}
          </li>
        ))}
      </ul>
    </div>
  )
}
