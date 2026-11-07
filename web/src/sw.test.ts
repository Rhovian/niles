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
  await import('./sw');
  let work!: Promise<void>;
  const waitUntil = (promise: Promise<void>) => { work = promise; };
  const deliver = async (session_id: string, reason = 'waiting', host?: { id: string; name: string }) => {
    listeners.get('push')!({ data: { json: () => ({ host, session_id, project: 'p', reason }) }, waitUntil });
    await work;
  };
  const close = vi.fn();
  const click = async (data: unknown) => {
    listeners.get('notificationclick')!({ notification: { close, data }, waitUntil });
    await work;
  };
  await deliver('s');
  expect(showNotification).toHaveBeenLastCalledWith('Niles · p', expect.objectContaining({ body: 'p: waiting', data: { session_id: 's' }, tag: 'session:s', renotify: false }));
  await deliver('s', 'Operator sent a message');
  expect(showNotification).toHaveBeenLastCalledWith('Niles · p', expect.objectContaining({ body: 'p: Operator sent a message', tag: 'session:s', renotify: false }));
  await deliver('other');
  expect(showNotification).toHaveBeenLastCalledWith('Niles · p', expect.objectContaining({ tag: 'session:other', data: { session_id: 'other' } }));
  await deliver('p:20261022T173949644004000Z');
  expect(showNotification).toHaveBeenLastCalledWith('Niles · p', expect.objectContaining({ tag: 'session:p:20261022T173949644004000Z', data: { session_id: 'p:20261022T173949644004000Z' } }));
  for (const host of [{ id: 'desk', name: 'Desktop' }, { id: 'laptop', name: 'Laptop' }]) {
    const id = JSON.stringify([host.id, 's']);
    await deliver('s', 'waiting', host);
    expect(showNotification).toHaveBeenLastCalledWith(`Niles · ${host.name} · p`, expect.objectContaining({ tag: `session:${id}`, data: { session_id: id } }));
    await click(showNotification.mock.lastCall![1].data);
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
  await deliver('');
  expect(showNotification).toHaveBeenLastCalledWith('Niles', expect.objectContaining({ body: 'A session needs your attention.' }));
  await click({ session_id: 'https://evil.test' });
  expect(close).toHaveBeenCalled();
  expect(navigate).toHaveBeenCalledWith('https://niles.test/#/sessions/https%3A%2F%2Fevil.test');
  expect(focus).toHaveBeenCalled();
  expect(openWindow).not.toHaveBeenCalled();
  matchAll.mockResolvedValueOnce([]);
  await click({ session_id: 'project:session' });
  expect(openWindow).toHaveBeenCalledWith('https://niles.test/#/sessions/project%3Asession');
});

it('serves the installed shell offline through Workbox and leaves every API request on the network', async () => {
  vi.doUnmock('workbox-precaching'); vi.doUnmock('workbox-routing'); vi.resetModules();
  class LifetimeEvent extends Event {
    work: Promise<unknown>[] = [];
    waitUntil(promise: Promise<unknown>) { this.work.push(promise); }
  }
  class RequestEvent extends LifetimeEvent {
    response?: Promise<Response>;
    constructor(readonly request: Request) { super('fetch'); }
    respondWith(response: Promise<Response>) { this.response = Promise.resolve(response); }
  }
  vi.stubGlobal('__WB_DISABLE_DEV_LOGS', true);
  const origin = location.origin;
  const BrowserRequest = Request;
  vi.stubGlobal('Request', class extends BrowserRequest {
    constructor(input: RequestInfo | URL, options?: RequestInit) {
      super(typeof input === 'string' ? new URL(input, location.href) : input, options);
    }
  });
  const stored = new Map<string, Map<string, Response>>();
  const cache = (name: string) => {
    if (!stored.has(name)) stored.set(name, new Map());
    const entries = stored.get(name)!;
    const key = (input: RequestInfo | URL) => new URL(input instanceof Request ? input.url : String(input), origin).href;
    return {
      match: async (input: RequestInfo | URL) => entries.get(key(input))?.clone(),
      put: async (input: RequestInfo | URL, response: Response) => { entries.set(key(input), response.clone()); },
    };
  };
  vi.stubGlobal('caches', {
    open: async (name: string) => cache(name),
    match: async (input: RequestInfo | URL, options: MultiCacheQueryOptions) => cache(options.cacheName!).match(input),
  });
  vi.stubGlobal('ExtendableEvent', LifetimeEvent); vi.stubGlobal('FetchEvent', RequestEvent);
  const worker = Object.assign(new EventTarget(), {
    __WB_MANIFEST: [{ url: '/index.html', revision: 'shell' }, { url: '/assets/app.js', revision: 'script' }],
    __WB_DISABLE_DEV_LOGS: true, caches, location, registration: { scope: `${origin}/` },
  });
  vi.stubGlobal('self', worker);
  const network = vi.fn<typeof fetch>(async input => new Response(new URL(input instanceof Request ? input.url : String(input), origin).pathname));
  vi.stubGlobal('fetch', network);
  await import('./sw');
  const install = new LifetimeEvent('install'); worker.dispatchEvent(install);
  await Promise.all(install.work);
  network.mockRejectedValue(new TypeError('Tailnet disconnected'));
  for (const [path, mode, expected] of [['/', 'navigate', '/index.html'], ['/chat', 'navigate', '/index.html'], ['/assets/app.js', 'cors', '/assets/app.js']] as const) {
    const request = new Request(`${origin}${path}`);
    Object.defineProperty(request, 'mode', { value: mode });
    const event = new RequestEvent(request); worker.dispatchEvent(event);
    expect(await (await event.response!).text()).toBe(expected);
    await Promise.all(event.work);
  }
  const apiPaths = ['/hosts.json', '/hosts.json?refresh=1', '/api?refresh=1', '/h?refresh=1', '/api/sessions', '/api/telemetry', '/h/desk/api/sessions'];
  for (const [path, mode, method] of [...apiPaths.map(path => [path, 'navigate', 'GET']), ['/api/sessions', 'cors', 'GET'], ['/api/sessions/solo/message', 'cors', 'POST']]) {
    const request = new Request(`${origin}${path}`, { method });
    Object.defineProperty(request, 'mode', { value: mode });
    const event = new RequestEvent(request); worker.dispatchEvent(event);
    expect(event.response, `${mode} ${path}`).toBeUndefined();
    await expect(fetch(request)).rejects.toThrow('Tailnet disconnected');
  }
});
