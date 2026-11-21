// Browser notifications for what the inbox hears while this tab is open
// but not in front (ADR 0028).

import { useQuery } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useEffect, useRef } from 'react'
import { api, type InboxEntry } from './api'
import { keys } from './query'
import { hosted, onLan } from './served'

const POLL_MS = 15_000

/** What each kind of notification is about, for settings and titles. */
export const KIND_LABEL: Record<string, string> = {
  permission: 'An agent asks for a permission',
  plan: 'An agent wrote a plan to approve',
  errored: 'An agent stopped with an error',
  offline: 'A machine went offline with an agent at work',
  finished: 'An agent finished its turn',
  recovered: 'A recovery thread was made',
  mr_opened: 'A merge request into your thread',
  mr_approved: 'Your merge request was merged',
  mr_declined: 'Your merge request was declined',
  mr_changes_requested: 'Changes asked of your merge request',
  comment: 'A comment on your thread',
  reply: 'A reply to your comment',
  mention: 'You were mentioned',
}

export function notificationsAllowed(): boolean {
  return typeof Notification !== 'undefined' && Notification.permission === 'granted'
}

/** Pushes reach this browser, so an open tab needn't notify again. */
let pushing = false

/** `applicationServerKey` takes the key as bytes, the server gives base64url. */
function keyBytes(key: string): Uint8Array<ArrayBuffer> {
  const base64 = key.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (key.length % 4)) % 4)
  return Uint8Array.from(atob(base64), (char) => char.charCodeAt(0))
}

/** Whether this browser can take pushes at all: a phone's needs Silt on its Home Screen on iOS. */
export function pushable(): boolean {
  return 'serviceWorker' in navigator && 'PushManager' in window && typeof Notification !== 'undefined'
}

/**
 * Subscribes this browser to pushes, when notifications are allowed: the server's, when it serves the app, or the
 * machine's own, on a phone paired over the network (ADR 0112).
 */
export async function subscribeToPush(): Promise<void> {
  if (!(hosted() || onLan()) || !notificationsAllowed() || !pushable()) return
  const registration = await navigator.serviceWorker.register('sw.js')
  const key = onLan() ? (await api.lanMe()).push_key : await api.pushKey()
  if (!key) return
  const subscription =
    (await registration.pushManager.getSubscription()) ??
    (await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(key) }))
  if (onLan()) await api.lanPush(subscription.toJSON())
  else await api.subscribePush(subscription.toJSON())
  pushing = true
}

/** An item is new again when the server brings it up to date. */
const keyOf = (item: InboxEntry) => `${item.server ?? ''}:${item.id}:${item.created_ms}`

/** Shows a browser notification for each new unread item whose kind notifies. */
export function useNotifier() {
  const navigate = useNavigate()
  const inbox = useQuery({ queryKey: keys.inbox(false), queryFn: () => api.inbox(false), refetchInterval: POLL_MS, retry: false })
  const known = useRef<Set<string> | null>(null)
  useEffect(() => {
    subscribeToPush().catch(() => undefined)
  }, [])
  useEffect(() => {
    const items = inbox.data
    if (!items) return
    // What was there before this tab opened isn't news.
    if (known.current === null) {
      known.current = new Set(items.map(keyOf))
      return
    }
    for (const item of items) {
      if (known.current.has(keyOf(item))) continue
      known.current.add(keyOf(item))
      if (pushing || !item.notify || item.read || !notificationsAllowed() || document.hasFocus()) continue
      const shown = new Notification(KIND_LABEL[item.kind] ?? 'Silt', { body: item.text, tag: `silt-${item.id}` })
      shown.onclick = () => {
        window.focus()
        void navigate({ to: '/t/$id', params: { id: item.thread } })
        shown.close()
      }
    }
  }, [inbox.data, navigate])
}
