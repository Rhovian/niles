import { expect, it, vi } from 'vitest';
vi.mock('workbox-precaching', () => ({ precacheAndRoute: vi.fn(), cleanupOutdatedCaches: vi.fn(), createHandlerBoundToURL: vi.fn() }));
vi.mock('workbox-routing', () => ({ NavigationRoute: vi.fn(), registerRoute: vi.fn() }));
it('always displays a visible notification and opens only an encoded same-origin session', async () => {
  const listeners = new Map<string, (event: unknown) => void>();
  const showNotification = vi.fn().mockResolvedValue(undefined);
  const navigate = vi.fn().mockResolvedValue(undefined);
  const focus = vi.fn().mockResolvedValue(undefined);
  const openWindow = vi.fn();
  const matchAll = vi.fn().mockResolvedValue([{ url: 'https://niles.test/', navigate, focus }]);
  vi.stubGlobal('self', {
    addEventListener: (type: string, listener: (event: unknown) => void) => listeners.set(type, listener),
    registration: { showNotification }, location: { origin: 'https://niles.test' },
    clients: { matchAll, openWindow },
  });
  try {
    await import('./sw');
    let work!: Promise<void>;
    const waitUntil = (promise: Promise<void>) => { work = promise; };
    listeners.get('push')!({ data: { json: () => ({ session_id: 's', project: 'p', reason: 'waiting' }) }, waitUntil });
    await work;
    expect(showNotification).toHaveBeenLastCalledWith('Niles', expect.objectContaining({ body: 'p: waiting', data: { session_id: 's' } }));
    listeners.get('push')!({ data: { json: () => { throw new SyntaxError(); } }, waitUntil });
    await work;
    expect(showNotification).toHaveBeenLastCalledWith('Niles', expect.objectContaining({ body: 'A session needs your attention.' }));
    const close = vi.fn();
    listeners.get('notificationclick')!({ notification: { close, data: { session_id: 'https://evil.test' } }, waitUntil });
    await work;
    expect(close).toHaveBeenCalled();
    expect(navigate).toHaveBeenCalledWith('https://niles.test/#/sessions/https%3A%2F%2Fevil.test');
    expect(focus).toHaveBeenCalled();
    expect(openWindow).not.toHaveBeenCalled();
    matchAll.mockResolvedValueOnce([]);
    listeners.get('notificationclick')!({ notification: { close, data: { session_id: 'project:session' } }, waitUntil });
    await work;
    expect(openWindow).toHaveBeenCalledWith('https://niles.test/#/sessions/project%3Asession');
  } finally { vi.unstubAllGlobals(); }
});
