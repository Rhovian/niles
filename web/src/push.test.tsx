import { expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/preact';
import { Notifications } from './push';
import { selectedSession, sessionHash } from './notification';

it('registers the browser subscription only after permission', async () => {
  const { subscription, subscribe, permission } = browser();
  const fetch = vi.fn().mockResolvedValueOnce({ ok: true, json: async () => ({ public_key: 'AQID' }) }).mockResolvedValueOnce({ status: 200 });
  vi.stubGlobal('fetch', fetch);
  render(<Notifications hosts={[{ id: '', name: 'This host' }]} />);
  expect(permission).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button'));
  await screen.findByText('Notifications enabled');
  expect(permission).toHaveBeenCalledTimes(1);
  expect(subscribe.mock.calls[0][0]).toMatchObject({ userVisibleOnly: true });
  expect(subscribe.mock.calls[0][0].applicationServerKey).toBe('AQID');
  expect(fetch.mock.calls[1][0]).toBe('/api/push/subscribe');
  expect(JSON.parse(fetch.mock.calls[1][1].body)).toEqual({ ...subscription.toJSON(), host: null });
});
it('shows permission denial without contacting the server', async () => {
  browser({ permission: 'denied' });
  const fetch = vi.fn(); vi.stubGlobal('fetch', fetch);
  enable();
  expect((await screen.findByRole('alert')).textContent).toContain('not granted');
  expect(fetch).not.toHaveBeenCalled();
});
it('decodes only the session hash route and never treats an ID as a URL', () => {
  const id = 'project:session/part?x#🦀';
  expect(selectedSession(sessionHash(id))).toBe(id);
  expect(sessionHash('https://evil.test')).toBe('#/sessions/https%3A%2F%2Fevil.test');
  expect(selectedSession('#/sessions/%zz')).toBeUndefined();
  expect(selectedSession('#/unrelated')).toBeUndefined();
});

it('surfaces a subscription persistence failure and lets the user retry', async () => {
  const { subscribe } = browser({ existing: true, noKey: true });
  vi.stubGlobal('fetch', vi.fn().mockResolvedValueOnce({ ok: true, json: async () => ({ public_key: 'AQID' }) }).mockResolvedValueOnce({ status: 503 }));
  enable();
  expect((await screen.findByRole('alert')).textContent).toContain('503');
  expect((screen.getByRole('button') as HTMLButtonElement).disabled).toBe(false);
  expect(subscribe).not.toHaveBeenCalled();
});

function enable(hosts = [{ id: '', name: 'This host' }]) {
  render(<Notifications hosts={hosts} />); fireEvent.click(screen.getByRole('button'));
}

const hosts = [{ id: 'desk', name: 'Desktop' }, { id: 'laptop', name: 'Laptop' }];
function browser({ existing = false, key = 'AQID', permission: result = 'granted', noKey = false } = {}) {
  const subscription = { options: noKey ? {} : { applicationServerKey: Uint8Array.from(atob(key), char => char.charCodeAt(0)).buffer }, toJSON: () => ({ endpoint: 'https://fcm.googleapis.com/device', keys: { auth: 'auth', p256dh: 'key' } }) };
  const subscribe = vi.fn().mockResolvedValue(subscription);
  vi.stubGlobal('navigator', { serviceWorker: { getRegistration: async () => ({ active: {}, pushManager: { getSubscription: async () => existing ? subscription : null, subscribe } }) } });
  vi.stubGlobal('PushManager', class {});
  const permission = vi.fn().mockResolvedValue(result);
  vi.stubGlobal('Notification', { requestPermission: permission });
  return { subscription, subscribe, permission };
}
it.each([false, true])('registers one subscription with every host, reusing an existing subscription (%s)', async existing => {
  const { subscription, subscribe } = browser({ existing });
  const fetcher = vi.fn(async (_url, options) => options?.method ? { status: 200 } : { ok: true, json: async () => ({ public_key: 'AQID' }) });
  vi.stubGlobal('fetch', fetcher);
  enable(hosts);
  await screen.findByText('Notifications enabled');
  expect(subscribe).toHaveBeenCalledTimes(existing ? 0 : 1);
  for (const host of hosts) {
    expect(fetcher).toHaveBeenCalledWith(`/h/${host.id}/api/push/vapid`, expect.objectContaining({ cache: 'no-store' }));
    expect(fetcher).toHaveBeenCalledWith(`/h/${host.id}/api/push/subscribe`, expect.objectContaining({ body: JSON.stringify({ ...subscription.toJSON(), host }) }));
  }
});
it('rejects mismatched host keys before subscribing or saving', async () => {
  const { subscribe } = browser();
  const fetcher = vi.fn(async (url: string) => ({ ok: true, json: async () => ({ public_key: url.includes('desk') ? 'AQID' : 'BAUG' }) }));
  vi.stubGlobal('fetch', fetcher);
  enable(hosts);
  expect((await screen.findByRole('alert')).textContent).toContain('same VAPID key');
  expect(subscribe).not.toHaveBeenCalled(); expect(fetcher).toHaveBeenCalledTimes(2);
});
it('rejects a stale browser key before saving to hosts', async () => {
  browser({ existing: true, key: 'BAUG' });
  const fetcher = vi.fn(async () => ({ ok: true, json: async () => ({ public_key: 'AQID' }) }));
  vi.stubGlobal('fetch', fetcher);
  enable(hosts);
  expect((await screen.findByRole('alert')).textContent).toContain('different VAPID key');
  expect(fetcher).toHaveBeenCalledTimes(2);
});
it('attempts every registration and retries partial failures with the same subscription', async () => {
  const { subscribe } = browser({ existing: true });
  let fail = true;
  const fetcher = vi.fn(async (url: string, options) => options?.method
    ? { status: fail && url.includes('desk') ? 503 : 200 }
    : { ok: true, json: async () => ({ public_key: 'AQID' }) });
  vi.stubGlobal('fetch', fetcher);
  enable(hosts);
  expect((await screen.findByRole('alert')).textContent).toContain('Desktop: Error: API request failed (503)');
  expect(fetcher.mock.calls.filter(([, options]) => options?.method)).toHaveLength(2);
  fail = false; fireEvent.click(screen.getByRole('button'));
  await screen.findByText('Notifications enabled');
  expect(subscribe).not.toHaveBeenCalled();
});
it('reports an asleep host during key discovery without registering a subset', async () => {
  const { subscribe } = browser();
  const fetcher = vi.fn(async (url: string) => ({ ok: !url.includes('laptop'), status: 502, json: async () => ({ public_key: 'AQID' }) }));
  vi.stubGlobal('fetch', fetcher);
  enable(hosts);
  expect((await screen.findByRole('alert')).textContent).toContain('Laptop: API request failed (502)');
  expect(subscribe).not.toHaveBeenCalled(); expect(fetcher).toHaveBeenCalledTimes(2);
});
