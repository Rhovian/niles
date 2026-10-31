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
    const deliver = async (session_id: string, reason = 'waiting') => {
      listeners.get('push')!({ data: { json: () => ({ session_id, project: 'p', reason }) }, waitUntil });
      await work;
    };
    await deliver('s');
    expect(showNotification).toHaveBeenLastCalledWith('Niles · p', expect.objectContaining({ body: 'p: waiting', data: { session_id: 's' }, tag: 'session:s', renotify: false }));
    await deliver('s', 'waiting again');
    expect(showNotification).toHaveBeenLastCalledWith('Niles · p', expect.objectContaining({ body: 'p: waiting again', tag: 'session:s', renotify: false }));
    await deliver('other');
    expect(showNotification).toHaveBeenLastCalledWith('Niles · p', expect.objectContaining({ tag: 'session:other', data: { session_id: 'other' } }));
    await deliver('p:20261022T173949644004000Z');
    expect(showNotification).toHaveBeenLastCalledWith('Niles · p', expect.objectContaining({ tag: 'session:p:20261022T173949644004000Z', data: { session_id: 'p:20261022T173949644004000Z' } }));
    for (const host of [{ id: 'desk', name: 'Desktop' }, { id: 'laptop', name: 'Laptop' }]) {
      const id = JSON.stringify([host.id, 's']);
      listeners.get('push')!({ data: { json: () => ({ host, session_id: 's', project: 'p', reason: 'waiting' }) }, waitUntil });
      await work;
      expect(showNotification).toHaveBeenLastCalledWith(`Niles · ${host.name} · p`, expect.objectContaining({ tag: `session:${id}`, data: { session_id: id } }));
      listeners.get('notificationclick')!({ notification: { close: vi.fn(), data: showNotification.mock.lastCall![1].data }, waitUntil });
      await work;
      expect(navigate).toHaveBeenLastCalledWith(`https://niles.test/#/sessions/${encodeURIComponent(id)}`);
    }
    // Worker restarts must not reset the notification identity.
    vi.resetModules();
    await import('./sw');
    await deliver('s');
    expect(showNotification).toHaveBeenLastCalledWith('Niles · p', expect.objectContaining({ tag: 'session:s', renotify: false }));
    listeners.get('push')!({ data: { json: () => { throw new SyntaxError(); } }, waitUntil });
    await work;
    expect(showNotification).toHaveBeenLastCalledWith('Niles', expect.objectContaining({ body: 'A session needs your attention.' }));
    expect(showNotification.mock.lastCall?.[1].tag).toBeUndefined();
    listeners.get('push')!({ data: { json: () => ({ session_id: '', project: 'p', reason: 'waiting' }) }, waitUntil });
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
