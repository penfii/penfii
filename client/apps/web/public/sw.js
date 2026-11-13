/*
 * Penfii service worker: Web Push and notification clicks, nothing else.
 *
 * It has no `fetch` handler and no cache: it never sits between the app and the network. Offline
 * reading and writing is a separate feature and is not done here.
 *
 * A push message is JSON made by the server: { v, id, kind, title, body, path }. It holds no message
 * text and no workspace name; `path` is where the Inbox would open the same item.
 */

const NAVIGATE_MESSAGE = 'penfii.navigate'

/** The message of a push event, or a generic one when it cannot be read. */
function readPush(event) {
  try {
    const data = event.data ? event.data.json() : null
    if (data && typeof data === 'object') return data
  } catch {
    // Fall through to the generic message.
  }
  return { v: 1, id: null, kind: 'unknown', title: 'Penfii', body: '', path: '/' }
}

/** Only a path inside this app is ever opened. */
function safePath(path) {
  return typeof path === 'string' && path.startsWith('/') && !path.startsWith('//') ? path : '/'
}

function splitPath(path) {
  const url = new URL(path, 'https://penfii.invalid')
  return { pathname: url.pathname.replace(/\/+$/, ''), search: url.searchParams }
}

/**
 * Whether a window is showing what the notification is about. A channel is also shown by the same
 * channel inside its Project; a Task is shown by its panel; a Doc by its page. Being in the app on
 * some other screen does not count.
 */
function isViewing(windowUrl, targetPath) {
  const here = splitPath(windowUrl)
  const target = splitPath(safePath(targetPath))
  const ticket = target.search.get('panel')
  if (ticket && ticket.startsWith('ticket:')) return here.search.get('panel') === ticket
  const doc = target.search.get('doc')
  if (doc) return here.pathname === target.pathname && here.search.get('doc') === doc
  if (here.pathname === target.pathname) return true
  // /w/{ws}/c/{conversation}[/t/{root}] is also /w/{ws}/projects/{p}/chat/{conversation}[/t/{root}]
  const channel = /^\/w\/([^/]+)\/c\/([^/]+)(\/t\/[^/]+)?$/.exec(target.pathname)
  if (channel) {
    const inProject = new RegExp(`^/w/${channel[1]}/projects/[^/]+/chat/${channel[2]}${channel[3] ?? ''}$`)
    return inProject.test(here.pathname)
  }
  return false
}

/**
 * WebKit (Safari, and every Home Screen app on iPhone and iPad) gives a subscription a small quota
 * of pushes that end without a visible notification and then removes the subscription, silently.
 * Chromium browsers (Chrome, Edge) have no such quota: they only need a visible window.
 */
function countsSilentPushes(userAgent) {
  const ua = String(userAgent || '')
  return /AppleWebKit/.test(ua) && !/(Chrome|Chromium|CriOS|Edg|EdgA|EdgiOS|OPR)\//.test(ua)
}

async function showUnlessViewing(event) {
  const message = readPush(event)
  const path = safePath(message.path)
  const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true })
  const viewing = windows.some((client) => client.visibilityState === 'visible' && client.focused && isViewing(client.url, path))
  // The Inbox and the open screen refresh themselves through the app's socket; nothing to show.
  const mustShow = viewing && countsSilentPushes(self.navigator && self.navigator.userAgent)
  if (viewing && !mustShow) return
  // One notification per Inbox item: a repeated push replaces it instead of stacking. (A push that
  // cannot be read has no item; the one shown only to satisfy WebKit still needs a tag to be found.)
  const tag = message.id ? `inbox:${message.id}` : viewing ? 'inbox:viewing' : undefined
  await self.registration.showNotification(String(message.title || 'Penfii'), {
    body: String(message.body || ''),
    icon: '/icon-192.png',
    badge: '/icon-192.png',
    tag,
    data: { path, id: message.id ?? null },
    // Shown only to satisfy WebKit's quota: no sound, and it is taken down again at once.
    silent: viewing,
  })
  if (viewing) {
    // It is already shown, so a failure to take it down must not fail the push event.
    try {
      for (const shown of await self.registration.getNotifications({ tag })) shown.close()
    } catch {
      // Left on screen; the person is looking at the same screen anyway.
    }
  }
}

async function openTarget(path) {
  const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true })
  const target = safePath(path)
  const client = windows.find((c) => c.focused) ?? windows.find((c) => c.visibilityState === 'visible') ?? windows[0]
  if (client) {
    await client.focus()
    // The app moves within itself (no reload); a window that cannot hear it is navigated.
    client.postMessage({ type: NAVIGATE_MESSAGE, path: target })
    return
  }
  await self.clients.openWindow(target)
}

// Only inside a service worker (the unit tests load this file in Node).
if (typeof ServiceWorkerGlobalScope !== 'undefined' && self instanceof ServiceWorkerGlobalScope) {
  self.addEventListener('push', (event) => {
    event.waitUntil(showUnlessViewing(event))
  })

  self.addEventListener('notificationclick', (event) => {
    event.notification.close()
    const path = event.notification.data && event.notification.data.path
    event.waitUntil(openTarget(path))
  })

}

// Lets the unit tests load this file.
if (typeof module !== 'undefined') module.exports = { readPush, safePath, isViewing, countsSilentPushes, showUnlessViewing, NAVIGATE_MESSAGE }
