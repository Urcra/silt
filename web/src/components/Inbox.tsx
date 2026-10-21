import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { api, short, type InboxRead } from '../api'
import { useTopic } from '../bus'
import { keys, message } from '../query'

export function Inbox() {
  useTopic('inbox')
  const [all, setAll] = useState(false)
  const queries = useQueryClient()
  const inbox = useQuery({ queryKey: keys.inbox(all), queryFn: () => api.inbox(all) })
  const notifications = inbox.data ?? []
  const markRead = async (read: Partial<InboxRead>) => {
    await api.readNotifications(read)
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
      {inbox.error && <p className="text-danger">{message(inbox.error)}</p>}
      {inbox.isSuccess && notifications.length === 0 && <p className="text-fg-3">Nothing new.</p>}
      <ul data-testid="inbox" className="space-y-2">
        {notifications.map((notification) => (
          <li key={notification.id} className={notification.read ? 'text-fg-3' : ''}>
            <Link className="text-accent-fg" to="/t/$id" params={{ id: notification.thread }}>
              {short(notification.thread)}
            </Link>{' '}
            {notification.text}{' '}
            {!notification.read && (
              <button
                className="text-xs text-accent-fg"
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
