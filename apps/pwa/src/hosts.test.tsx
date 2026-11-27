import { Stream, json, openMenu, deferred, fakeHosts, signInSettings, standaloneManifest } from './test-utils';
import { beforeEach, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/preact';
import { App } from './App';
import { configuredHosts, hostsSchema, qualified, HOST_TIMEOUT_MS, hostFailure } from './hosts';
import { API_NAVIGATION_PATH } from './shell';
import { ApiError } from './api';
import { apiFetch, authorizationUrl, finishSignIn, hostResource } from './auth';
import { createHash, webcrypto } from 'node:crypto';

const hosts = [{ id: 'desk', name: 'Desktop' }, { id: 'laptop', name: 'Laptop' }];
const session = { id: 'same/id', project: 'same project', agent: { family: 'codex', entry_skill: 'lead' }, waiting: false, worker_count: 1 };
const project = { name: session.project, state: 'running', workers: [{ id: 'same/worker', skill: 'worker', lost: false }] };

let server: ReturnType<typeof fakeHosts>;
const hostMenu = (name: string) => within(screen.getByRole('region', { name }));
async function openLead(name: string) {
  openMenu();
  const menu = hostMenu(name);
  fireEvent.click(await menu.findByRole('button', { name: project.name, exact: true }));
  fireEvent.click(menu.getByRole('button', { name: `Open ${project.name} lead chat` }));
  await screen.findByRole('region', { name: `Chat with ${project.name}` });
}
beforeEach(() => {
  server = fakeHosts({ hosts, defaults: {
    '/api/projects': { api_version: 1, projects: [project] }, '/api/sessions': { api_version: 1, sessions: [session] },
  } });
});
function stubLocks() {
  let held: Promise<unknown> = Promise.resolve();
  vi.stubGlobal('navigator', Object.create(navigator, { locks: { value: {
    request: (_key: string, task: () => unknown) => {
      const next = held.then(task);
      held = next.catch(() => undefined);
      return next;
    },
  } } }));
}

it('binds PKCE and state to the selected host for both exchange methods', async () => {
  vi.stubGlobal('crypto', webcrypto);
  // jsdom and Node WebCrypto use different ArrayBuffer realms.
  vi.stubGlobal('ArrayBuffer', (await webcrypto.subtle.digest('SHA-256', new Uint8Array())).constructor);
  stubLocks();
  const { issuer } = signInSettings;
  let auth: URL;
  let resource: string;
  const exchange = vi.fn();
  vi.stubGlobal('fetch', async (input: URL | string, options?: RequestInit) => {
    const url = String(input);
    if (url === `${issuer}/.well-known/openid-configuration`) return Response.json({
      issuer, authorization_endpoint: `${issuer}/authorize`, token_endpoint: `${issuer}/token`,
    });
    exchange();
    expect(options?.method).toBe('POST');
    const viaHost = url === '/h/desk/api/auth/token';
    const { code, code_verifier, ...request } = viaHost ? JSON.parse(String(options?.body))
      : Object.fromEntries(new URLSearchParams(options?.body as URLSearchParams));
    expect(code).toBe('session-code');
    expect(createHash('sha256').update(code_verifier).digest('base64url')).toBe(auth.searchParams.get('code_challenge'));
    if (viaHost) expect(request).toEqual({});
    else {
      expect(url).toBe(`${issuer}/token`);
      expect(request).toMatchObject({ resource, grant_type: 'authorization_code', client_id: 'phone' });
    }
    return Response.json({ token_type: 'Bearer', access_token: resource, refresh_token: 'refresh' });
  });
  for (const [id, client_id, host_token_exchange] of [['desk', 'desk-client', true], ['laptop', 'phone', false]] as const) {
    const prefix = `/h/${id}`;
    const settings = { issuer, client_id, host_token_exchange };
    resource = hostResource(prefix);
    auth = await authorizationUrl(prefix, settings);
    expect(auth.searchParams.get('resource')).toBe(resource);
    expect(auth.searchParams.get('client_id')).toBe(client_id);
    expect(auth.searchParams.get('code_challenge_method')).toBe('S256');
    if (host_token_exchange) {
      history.replaceState(null, '', '/?code=session-code&state=wrong');
      await expect(finishSignIn()).rejects.toThrow(/state/);
      expect(exchange).not.toHaveBeenCalled();
      auth = await authorizationUrl(prefix, settings);
    }
    history.replaceState(null, '', `/?code=session-code&state=${auth.searchParams.get('state')}`);
    await finishSignIn();
    expect(JSON.parse(localStorage.getItem(`niles.oauth.token:${resource}`)!)).toEqual(
      host_token_exchange ? { accessToken: resource, refreshToken: 'refresh' } : { accessToken: resource },
    );
  }
});

it('refreshes persisted credentials once for concurrent startup requests and prompts sign-in after rejection', async () => {
  stubLocks();
  const base = '/h/desk/api';
  const key = 'niles.oauth.token:http://localhost:3000/h/desk/api';
  localStorage.setItem(key, JSON.stringify({ accessToken: 'expired-token', refreshToken: 'first-refresh' }));
  sessionStorage.clear();
  const refresh = vi.fn().mockResolvedValueOnce(Response.json({
    token_type: 'Bearer', access_token: 'new-token', refresh_token: 'next-refresh',
  }));
  server.responses.set(`${base}/auth/token`, refresh);
  for (const [route, data] of [['sessions', { sessions: [session] }], ['projects', { projects: [project] }]] as const) {
    server.responses.set(`${base}/${route}`, options => new Headers(options?.headers).get('Authorization') === 'Bearer new-token'
      ? json({ api_version: 1, ...data }) : new Response(null, { status: 401 }));
  }
  render(<App />);
  await screen.findByText('Open projects on the sidebar.');
  await openLead('Desktop');
  server.responses.set(Stream.latest.url, options => {
    expect(new Headers(options?.headers).get('Authorization')).toBe('Bearer new-token');
    return new Response('');
  });
  expect((await Stream.latest.options.fetch(new URL(Stream.latest.url, location.origin))).ok).toBe(true);
  expect(refresh).toHaveBeenCalledOnce();
  expect(JSON.parse(refresh.mock.calls[0][0].body)).toEqual({ refresh_token: 'first-refresh' });
  expect(JSON.parse(localStorage.getItem(key)!).refreshToken).toBe('next-refresh');

  server.failHost('desk', new Response(null, { status: 401 }));
  server.responses.delete(`${base}/sessions`); server.responses.delete(`${base}/projects`);
  refresh.mockResolvedValueOnce(new Response(null, { status: 404 }));
  await expect(apiFetch(`${base}/sessions`)).rejects.toThrow('Host token exchange failed (HTTP 404)');
  refresh.mockResolvedValueOnce(new Response(null, { status: 400 }));
  fireEvent(window, new Event('focus'));
  await screen.findByRole('button', { name: 'Sign in to Desktop' });
  expect(refresh).toHaveBeenCalledTimes(3);
  expect(JSON.parse(refresh.mock.calls[2][0].body)).toEqual({ refresh_token: 'next-refresh' });
});

it('merges colliding projects, qualifies selection and routes chat, sends and status by host', async () => {
  const app = render(<App />);
  await screen.findByText('Open projects on the sidebar.');
  await openLead('Desktop');
  const first = Stream.opened.at(-1)!;
  expect(first.url).toBe('/h/desk/api/sessions/same%2Fid/events');
  first.emit('snapshot', { turns: [{ id: 'same-turn', role: 'agent', text: 'Desktop transcript' }], truncated: false });
  await screen.findByText('Desktop transcript');
  await openLead('Laptop');
  const second = Stream.opened.at(-1)!;
  expect(second.url).toBe('/h/laptop/api/sessions/same%2Fid/events');
  expect(first.close).toHaveBeenCalled();
  expect(screen.queryByText('Desktop transcript')).toBeNull();
  expect(localStorage.getItem('niles.activeProject')).toBe(qualified(hosts[1], project.name));
  second.emit('snapshot', { turns: [], truncated: false });
  await waitFor(() => expect(screen.getByRole('button', { name: 'Send' }).hasAttribute('disabled')).toBe(true));
  fireEvent.input(screen.getByLabelText('Message agent'), { target: { value: 'hello laptop' } });
  await waitFor(() => expect(screen.getByRole('button', { name: 'Send' }).hasAttribute('disabled')).toBe(false));
  fireEvent.click(screen.getByRole('button', { name: 'Send' }));
  await waitFor(() => expect(server.fetcher).toHaveBeenCalledWith('/h/laptop/api/sessions/same%2Fid/message', expect.objectContaining({ method: 'POST' })));
  second.emit('status', { ...session, agent: { ...session.agent, family: 'laptop-only' } });
  await screen.findByText('laptop-only');
  expect(Stream.opened).toHaveLength(2);
  await openLead('Desktop');
  expect(screen.getByText('codex')).toBeDefined();
  app.unmount(); render(<App />);
  await waitFor(() => expect(Stream.opened.at(-1)?.url).toBe(first.url));
});
it('routes worker chats and lifecycle actions without leaking qualified IDs to servers', async () => {
  vi.spyOn(window, 'confirm').mockReturnValue(true);
  render(<App />); await screen.findByText('Open projects on the sidebar.'); openMenu();
  const menu = hostMenu('Laptop');
  fireEvent.click(menu.getByRole('button', { name: project.name, exact: true }));
  fireEvent.click(menu.getByRole('button', { name: 'Stop same project' }));
  await waitFor(() => expect(server.fetcher).toHaveBeenCalledWith('/h/laptop/api/projects/same%20project/session', expect.objectContaining({ method: 'DELETE', cache: 'no-store' })));
  fireEvent.click(menu.getByRole('button', { name: 'same/worker', exact: true }));
  fireEvent.click(menu.getByRole('button', { name: 'Close same/worker' }));
  await waitFor(() => expect(server.fetcher).toHaveBeenCalledWith('/h/laptop/api/projects/same%20project/workers/same%2Fworker', expect.objectContaining({ method: 'DELETE', cache: 'no-store' })));
  fireEvent.click(menu.getByRole('button', { name: 'Open same/worker chat' }));
  await waitFor(() => expect(Stream.opened.at(-1)?.url).toBe('/h/laptop/api/projects/same%20project/workers/same%2Fworker/events'));
});
it.each([new ApiError(502), new TypeError('Network down')])('isolates host failure %s and recovers on focus', async failure => {
  server.failHost('laptop', failure);
  render(<App />);
  const asleep = failure instanceof ApiError;
  expect((await screen.findByRole('alert')).textContent).toContain(asleep ? 'Laptop: Asleep' : 'Reconnect Tailscale');
  openMenu();
  expect(hostMenu('Desktop').getByRole('button', { name: project.name, exact: true })).toBeDefined();
  server.failHost('laptop'); fireEvent(window, new Event('focus'));
  await hostMenu('Laptop').findByRole('button', { name: project.name, exact: true });
  expect(screen.queryByRole('alert')).toBeNull();
});
it.each([
  ['projects', { api_version: 99, projects: 'new schema' }, 'Laptop: Update the web app. API version 99'],
  ['sessions', { api_version: 99, sessions: 'new schema' }, 'Laptop: Update the web app. API version 99'],
  ['sessions', { api_version: 1, sessions: 'invalid' }, 'Laptop: Invalid API response'],
] as const)('isolates incompatible or malformed %s payloads (%j)', async (endpoint, data, message) => {
  server.responses.set(`/h/laptop/api/${endpoint}`, json(data));
  render(<App />);
  expect((await screen.findByRole('alert')).textContent).toContain(message);
  await openLead('Desktop');
  expect(Stream.opened.at(-1)?.url).toContain('/h/desk/');
});
it.each([json({ api_version: 99 }), new TypeError('Network down')])('clears stale chat when its host becomes unavailable (%s)', async failure => {
  render(<App />); await screen.findByText('Open projects on the sidebar.'); await openLead('Laptop');
  const stream = Stream.opened.at(-1)!;
  server.failHost('laptop', failure); fireEvent(window, new Event('focus'));
  await screen.findByRole('alert');
  expect(screen.queryByLabelText('Message agent')).toBeNull();
  await waitFor(() => expect(stream.close).toHaveBeenCalled());
});
it('does not let a slow host block another host or overlap its refresh', async () => {
  const { promise, resolve: finish } = deferred<Response>();
  server.responses.set('/h/laptop/api/sessions', promise);
  render(<App />); await screen.findByText('Open projects on the sidebar.'); await openLead('Desktop');
  fireEvent(window, new Event('focus'));
  expect(server.fetcher.mock.calls.filter(([url]) => url === '/h/laptop/api/sessions')).toHaveLength(1);
  await act(async () => { finish(json({ api_version: 1, sessions: [session] })); });
});
it.each([
  { hosts: [{ ...standaloneManifest.hosts[0], id: '../evil' }] },
  { hosts: [standaloneManifest.hosts[0], standaloneManifest.hosts[0]] },
  { hosts: [{ ...standaloneManifest.hosts[0], name: '' }] },
  { hosts: [] },
  { hosts: [{ ...standaloneManifest.hosts[0], id: 'desk' }, ...standaloneManifest.hosts] },
  { hosts },
  { hosts: hosts.map(host => ({ ...host, client_id: 'shared' })), host_token_exchange: true },
  { hosts: [{ ...standaloneManifest.hosts[0], client_id: ' ' }] },
  { issuer: 'http://login.example.test' },
])('rejects malformed manifests without making host requests (%j)', async fields => {
  const manifest = { ...standaloneManifest, ...fields };
  server.fetcher.mockResolvedValue(json(manifest));
  render(<App />); await screen.findByRole('alert');
  expect(server.fetcher).toHaveBeenCalledTimes(1);
  expect(hostsSchema.safeParse(manifest).success).toBe(false);
});
it('requires a manifest even for a direct single host', async () => {
  server.fetcher.mockResolvedValue(new Response(null, { status: 404 }));
  render(<App />);
  expect((await screen.findByRole('alert')).textContent).toContain('Cannot load hosts.json (HTTP 404)');
  expect(server.fetcher).toHaveBeenCalledTimes(1);
  server.fetcher.mockResolvedValue(json(standaloneManifest));
  expect(await configuredHosts(new AbortController().signal)).toEqual(standaloneManifest);
  expect(qualified(hosts[0], 'a/b')).not.toBe(qualified(hosts[1], 'a/b'));
  for (const path of ['/h/desk/api/sessions', '/hosts.json']) expect(API_NAVIGATION_PATH.test(path)).toBe(true);
});

it('opens a qualified session link on the correct host', async () => {
  window.history.replaceState(null, '', `/#/sessions/${encodeURIComponent(qualified(hosts[1], session.id))}`);
  render(<App />);
  await waitFor(() => expect(Stream.opened.at(-1)?.url).toBe('/h/laptop/api/sessions/same%2Fid/events'));
  expect(localStorage.getItem('niles.activeProject')).toBe(qualified(hosts[1], project.name));
});
it('times out a hung host as asleep while keeping other hosts usable', async () => {
  const timer = vi.spyOn(globalThis, 'setTimeout');
  server.failHost('laptop', options => new Promise((_, reject) =>
    options!.signal!.addEventListener('abort', () => reject(options!.signal!.reason))));
  render(<App />); await screen.findByText('Open projects on the sidebar.');
  const timeouts = timer.mock.calls.filter(([, delay]) => delay === HOST_TIMEOUT_MS);
  expect(timeouts).toHaveLength(2);
  await act(async () => { (timeouts[1][0] as () => void)(); });
  await screen.findByText(/Laptop: Asleep/);
  openMenu();
  expect(hostMenu('Desktop').getByRole('button', { name: project.name, exact: true })).toBeDefined();
});
it('preserves newer SSE status only on its own host during concurrent refresh', async () => {
  render(<App />); await screen.findByText('Open projects on the sidebar.'); await openLead('Laptop');
  const { promise, resolve: finish } = deferred<Response>();
  server.responses.set('/h/laptop/api/sessions', promise);
  fireEvent(window, new Event('focus'));
  Stream.opened.at(-1)!.emit('status', { ...session, agent: { ...session.agent, family: 'new-status' } });
  await screen.findByText('new-status');
  await act(async () => { finish(json({ api_version: 1, sessions: [session] })); });
  expect(screen.getByText('new-status')).toBeDefined();
});
it.each([
  [new TypeError('Network down'), { state: 'unreachable' }],
  [new DOMException('timed out', 'TimeoutError'), { state: 'asleep' }],
  ...[502, 503, 504].map(status => [new ApiError(status), { state: 'asleep' }] as const),
  ...[401, 403, 500].map(status => [new ApiError(status), { state: 'error', error: `API request failed (${status})` }] as const),
] as const)('classifies host failure %s', (cause, expected) => {
  expect(hostFailure(cause)).toEqual(expected);
});

it('shows one origin banner when every host loses its connection', async () => {
  for (const host of hosts) server.failHost(host.id, new TypeError('Failed to fetch'));
  render(<App />);
  await screen.findByText(/Reconnect Tailscale/);
  expect(screen.getAllByRole('alert')).toHaveLength(1);
  openMenu(); fireEvent.click(screen.getByRole('button', { name: 'Telemetry', exact: true }));
  await screen.findByText(/Reconnect Tailscale/);
  expect(screen.getAllByRole('alert')).toHaveLength(1);
  for (const host of hosts) server.failHost(host.id);
  openMenu(); fireEvent.click(screen.getByRole('button', { name: 'Help', exact: true }));
  fireEvent(window, new Event('focus'));
  await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
});
