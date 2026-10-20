import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/preact';
import { Notifications } from './push';
import { App } from './App';
import { selectedSession, sessionHash } from './notification';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); window.history.replaceState(null, '', '/'); });
it('registers the browser subscription only after permission', async () => {
  const subscription = { toJSON: () => ({ endpoint: 'https://fcm.googleapis.com/device', keys: { auth: 'auth', p256dh: 'key' } }) };
  const subscribe = vi.fn().mockResolvedValue(subscription);
  const registration = { active: {}, pushManager: { getSubscription: vi.fn().mockResolvedValue(null), subscribe } };
  vi.stubGlobal('navigator', { serviceWorker: { getRegistration: vi.fn().mockResolvedValue(registration) } });
  vi.stubGlobal('PushManager', class {});
  const permission = vi.fn().mockResolvedValue('granted');
  vi.stubGlobal('Notification', { requestPermission: permission });
  const fetch = vi.fn().mockResolvedValueOnce({ ok: true, json: async () => ({ public_key: 'AQID' }) }).mockResolvedValueOnce({ status: 200 });
  vi.stubGlobal('fetch', fetch);
  render(<Notifications />);
  expect(permission).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button'));
  await screen.findByText('Notifications enabled');
  expect(permission).toHaveBeenCalledTimes(1);
  expect(subscribe.mock.calls[0][0]).toMatchObject({ userVisibleOnly: true });
  expect(subscribe.mock.calls[0][0].applicationServerKey).toBe('AQID');
  expect(fetch.mock.calls[1][0]).toBe('/api/push/subscribe');
  expect(JSON.parse(fetch.mock.calls[1][1].body)).toEqual(subscription.toJSON());
});
it('shows permission denial without contacting the server', async () => {
  vi.stubGlobal('navigator', { serviceWorker: {} });
  vi.stubGlobal('PushManager', class {});
  vi.stubGlobal('Notification', { requestPermission: vi.fn().mockResolvedValue('denied') });
  const fetch = vi.fn(); vi.stubGlobal('fetch', fetch);
  render(<Notifications />);
  fireEvent.click(screen.getByRole('button'));
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
  const subscription = { toJSON: () => ({ endpoint: 'device' }) };
  const subscribe = vi.fn();
  vi.stubGlobal('navigator', { serviceWorker: { getRegistration: async () => ({ active: {}, pushManager: { getSubscription: async () => subscription, subscribe } }) } });
  vi.stubGlobal('PushManager', class {});
  vi.stubGlobal('Notification', { requestPermission: async () => 'granted' });
  vi.stubGlobal('fetch', vi.fn().mockResolvedValueOnce({ ok: true, json: async () => ({ public_key: 'AQID' }) }).mockResolvedValueOnce({ status: 503 }));
  render(<Notifications />);
  fireEvent.click(screen.getByRole('button'));
  expect((await screen.findByRole('alert')).textContent).toContain('503');
  expect((screen.getByRole('button') as HTMLButtonElement).disabled).toBe(false);
  expect(subscribe).not.toHaveBeenCalled();
});

it('opens the session selected by a notification hash on initial app load', async () => {
  window.history.replaceState(null, '', '/#/sessions/project%3Asession');
  vi.stubGlobal('EventSource', class extends EventTarget { close() {} });
  Element.prototype.scrollIntoView = vi.fn();
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => ({ api_version: 1, sessions: [{ id: 'project:session', project: 'project', agent: { family: 'codex', entry_skill: 'solo' }, waiting: true, worker_count: 0 }] }) }));
  render(<App />);
  expect(await screen.findByLabelText('Message agent')).toBeDefined();
});
