/// <reference lib="webworker" />
import { cleanupOutdatedCaches, createHandlerBoundToURL, precacheAndRoute } from 'workbox-precaching';
import { NavigationRoute, registerRoute } from 'workbox-routing';
import { API_NAVIGATION_PATH } from './shell';
declare const self: ServiceWorkerGlobalScope & { __WB_MANIFEST: { url: string; revision: string | null }[] };

precacheAndRoute(self.__WB_MANIFEST);
cleanupOutdatedCaches();
registerRoute(new NavigationRoute(createHandlerBoundToURL('/index.html'), { denylist: [API_NAVIGATION_PATH] }));
// Payload parsing, notification contents and session deep links belong to #318.
self.addEventListener('push', event => {
  event.waitUntil(self.registration.showNotification('Niles', { body: 'A session needs your attention.', icon: '/icon-192.png' }));
});
self.addEventListener('notificationclick', event => {
  event.notification.close();
  event.waitUntil((async () => {
    const windows = await self.clients.matchAll({ type: 'window' });
    const existing = windows.find(client => client.url === new URL('/', self.location.origin).href);
    if (existing) await existing.focus();
    else await self.clients.openWindow('/');
  })());
});
