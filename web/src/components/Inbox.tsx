import { useState } from 'react'
import { api, short } from '../api'
import { href } from '../router'
import { useLoad } from '../useLoad'

export function Inbox() {
  const [all, setAll] = useState(false)
  const [version, setVersion] = useState(0)
  const inbox = useLoad(() => api.inbox(all), `inbox:${all}:${version}`)
  const notifications = inbox.data ?? []
  const markRead = async (read: { ids?: number[]; all?: boolean }) => {
    await api.readNotifications(read)
    setVersion(version + 1)
  }
  return (
    <div>
      <div className="mb-3 flex items-center gap-4">
        <h2 className="text-lg font-semibold">Inbox</h2>
        <label className="text-sm">
          <input type="checkbox" checked={all} onChange={(event) => setAll(event.target.checked)} /> show read
        </label>
        <button className="text-sm text-sky-700" onClick={() => void markRead({ all: true })}>
          Mark all read
        </button>
      </div>
      {inbox.error && <p className="text-rose-700">{inbox.error}</p>}
      {!inbox.error && notifications.length === 0 && <p className="text-neutral-500">Nothing new.</p>}
      <ul data-testid="inbox" className="space-y-2">
        {notifications.map((notification) => (
          <li key={notification.id} className={notification.read ? 'text-neutral-500' : ''}>
            <a className="text-sky-700" href={href({ page: 'thread', id: notification.thread })}>
              {short(notification.thread)}
            </a>{' '}
            {notification.text}{' '}
            {!notification.read && (
              <button
                className="text-xs text-sky-700"
                aria-label={`mark ${notification.id} read`}
                onClick={() => void markRead({ ids: [notification.id] })}
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
