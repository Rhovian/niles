/// <reference lib="webworker" />
import { cleanupOutdatedCaches, createHandlerBoundToURL, precacheAndRoute } from 'workbox-precaching';
import { NavigationRoute, registerRoute } from 'workbox-routing';
import { API_NAVIGATION_PATH } from './shell';
import { pushPayload, sessionHash } from './notification';
declare const self: ServiceWorkerGlobalScope & { __WB_MANIFEST: { url: string; revision: string | null }[] };

precacheAndRoute(self.__WB_MANIFEST);
cleanupOutdatedCaches();
registerRoute(new NavigationRoute(createHandlerBoundToURL('/index.html'), { denylist: [API_NAVIGATION_PATH] }));
self.addEventListener('push', event => {
  event.waitUntil((async () => {
    // Every push must display a notification, even when its payload is malformed.
    let input: unknown;
    try { input = event.data?.json(); } catch { input = undefined; }
    const parsed = pushPayload.safeParse(input);
    const title = parsed.success ? `Niles · ${parsed.data.session_id}` : 'Niles';
    const options = parsed.success ? {
      body: `${parsed.data.project}: ${parsed.data.reason}`,
      // Browser-owned replacement survives worker and server restarts.
      tag: `session:${parsed.data.session_id}`,
      data: { session_id: parsed.data.session_id },
    } : { body: 'A session needs your attention.' };
    // TypeScript 5.9's lib.webworker.d.ts omits this standard option.
    // https://notifications.spec.whatwg.org/#dictdef-notificationoptions
    const notificationOptions: NotificationOptions & { renotify: boolean } = {
      ...options, icon: '/icon-192.png', renotify: false,
    };
    await self.registration.showNotification(title, notificationOptions);
  })());
});
self.addEventListener('notificationclick', event => {
  event.notification.close();
  event.waitUntil((async () => {
    const id: unknown = event.notification.data?.session_id;
    const target = new URL(typeof id === 'string' ? `/${sessionHash(id)}` : '/', self.location.origin).href;
    const windows = await self.clients.matchAll({ type: 'window' });
    const existing = windows.find(client => new URL(client.url).origin === self.location.origin);
    if (existing) { await existing.navigate(target); await existing.focus(); }
    else await self.clients.openWindow(target);
  })());
});
