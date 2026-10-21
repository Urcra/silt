// The hosted app's service worker (ADR 0028): it shows what the server
// pushes, and opens the thread a notification is about.

self.addEventListener('push', (event) => {
  const data = event.data ? event.data.json() : {}
  event.waitUntil(
    self.registration.showNotification(data.title ?? 'Silt', { body: data.body, tag: data.tag, data: { url: data.url } }),
  )
})

self.addEventListener('notificationclick', (event) => {
  event.notification.close()
  const url = new URL(event.notification.data?.url ?? '', self.registration.scope).toString()
  event.waitUntil(
    self.clients.matchAll({ type: 'window', includeUncontrolled: true }).then((windows) => {
      const open = windows.find((window) => window.url.startsWith(self.registration.scope))
      if (!open) return self.clients.openWindow(url)
      return open.focus().then(() => open.navigate(url))
    }),
  )
})
