import { beforeEach, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/preact';
import { App } from './App';
import { configuredHosts, hostsSchema, qualified } from './hosts';
import { API_NAVIGATION_PATH } from './shell';

const hosts = [{ id: 'desk', name: 'Desktop' }, { id: 'laptop', name: 'Laptop' }];
const session = { id: 'same/id', project: 'same project', agent: { family: 'codex', entry_skill: 'lead' }, waiting: false, worker_count: 1 };
const project = { name: session.project, state: 'running', workers: [{ id: 'same/worker', skill: 'worker', lost: false }] };
const json = (data: unknown) => ({ ok: true, status: 200, json: async () => data }) as Response;
class Stream extends EventTarget {
  static opened: Stream[];
  close = vi.fn();
  constructor(readonly url: string) { super(); Stream.opened.push(this); }
  emit(name: string, data: unknown) { this.dispatchEvent(new MessageEvent(name, { data: JSON.stringify(data) })); }
}
let failures: Map<string, Response | Error>;
let fetcher: ReturnType<typeof vi.fn<typeof fetch>>;
const openMenu = () => fireEvent.click(screen.getByRole('button', { name: 'Open menu' }));
const hostMenu = (name: string) => within(screen.getByRole('region', { name }));
async function openLead(name: string) {
  openMenu();
  const menu = hostMenu(name);
  fireEvent.click(await menu.findByRole('button', { name: project.name, exact: true }));
  fireEvent.click(menu.getByRole('button', { name: `Open ${project.name} lead chat` }));
  await screen.findByRole('region', { name: `Chat with ${project.name}` });
}
beforeEach(() => {
  localStorage.clear(); window.history.replaceState(null, '', '/');
  Stream.opened = []; failures = new Map();
  vi.stubGlobal('EventSource', Stream);
  fetcher = vi.fn(async (input, options) => {
    const url = String(input);
    if (url === '/hosts.json') return json({ hosts });
    const host = hosts.find(host => url.startsWith(`/h/${host.id}/`))!;
    const failure = failures.get(host?.id);
    if (failure instanceof Error) throw failure;
    if (failure) return failure;
    if (options?.method) return json({});
    return json(url.endsWith('/projects') ? { api_version: 1, projects: [project] } : { api_version: 1, sessions: [session] });
  });
  vi.stubGlobal('fetch', fetcher);
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
  await waitFor(() => expect(fetcher).toHaveBeenCalledWith('/h/laptop/api/sessions/same%2Fid/message', expect.objectContaining({ method: 'POST' })));
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
  await waitFor(() => expect(fetcher).toHaveBeenCalledWith('/h/laptop/api/projects/same%20project/session', { method: 'DELETE', cache: 'no-store' }));
  fireEvent.click(menu.getByRole('button', { name: 'same/worker', exact: true }));
  fireEvent.click(menu.getByRole('button', { name: 'Close same/worker' }));
  await waitFor(() => expect(fetcher).toHaveBeenCalledWith('/h/laptop/api/projects/same%20project/workers/same%2Fworker', { method: 'DELETE', cache: 'no-store' }));
  fireEvent.click(menu.getByRole('button', { name: 'Open same/worker chat' }));
  await waitFor(() => expect(Stream.opened.at(-1)?.url).toBe('/h/laptop/api/projects/same%20project/workers/same%2Fworker/events'));
});
it.each([new TypeError('Network down'), { ok: false, status: 502 }, { ok: false, status: 503 }, { ok: false, status: 504 }])('isolates an unreachable host and recovers on focus (%s)', async failure => {
  failures.set('laptop', failure as Response | Error);
  render(<App />); await screen.findByText(/Laptop: Asleep/); openMenu();
  const wake = hostMenu('Laptop').getByRole('button', { name: 'Wake Laptop' });
  expect(wake.hasAttribute('disabled')).toBe(true);
  expect(hostMenu('Desktop').getByRole('button', { name: project.name, exact: true })).toBeDefined();
  failures.clear(); fireEvent(window, new Event('focus'));
  await hostMenu('Laptop').findByRole('button', { name: project.name, exact: true });
  expect(screen.queryByText(/Laptop: Asleep/)).toBeNull();
});
it.each([401, 403, 500])('reports HTTP %s as an error rather than asleep', async status => {
  failures.set('laptop', { ok: false, status } as Response);
  render(<App />);
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toContain(String(status));
  expect(alert.textContent).not.toContain('Asleep');
});
it.each(['projects', 'sessions'])('checks the %s API version per host before parsing incompatible payloads', async endpoint => {
  const implementation = fetcher.getMockImplementation()!;
  fetcher.mockImplementation((url, options) => String(url) === `/h/laptop/api/${endpoint}`
    ? Promise.resolve(json({ api_version: 99, projects: 'new schema' })) : implementation(url, options));
  render(<App />); await screen.findByText(/Laptop: Update the web app. API version 99/);
  await openLead('Desktop');
  expect(Stream.opened.at(-1)?.url).toContain('/h/desk/');
});
it.each([json({ api_version: 99 }), new TypeError('Network down')])('clears stale chat when its host becomes unavailable (%s)', async failure => {
  render(<App />); await screen.findByText('Open projects on the sidebar.'); await openLead('Laptop');
  const stream = Stream.opened.at(-1)!;
  failures.set('laptop', failure); fireEvent(window, new Event('focus'));
  await screen.findByRole('alert');
  expect(screen.queryByLabelText('Message agent')).toBeNull();
  await waitFor(() => expect(stream.close).toHaveBeenCalled());
});
it('does not let a slow host block another host or overlap its refresh', async () => {
  const implementation = fetcher.getMockImplementation()!;
  let finish!: (value: Response) => void;
  fetcher.mockImplementation((url, options) => String(url) === '/h/laptop/api/sessions'
    ? new Promise(done => { finish = done; }) : implementation(url, options));
  render(<App />); await screen.findByText('Open projects on the sidebar.'); await openLead('Desktop');
  fireEvent(window, new Event('focus'));
  expect(fetcher.mock.calls.filter(([url]) => url === '/h/laptop/api/sessions')).toHaveLength(1);
  await act(async () => { finish(json({ api_version: 1, sessions: [session] })); });
});
it.each([{ hosts: [{ id: '../evil', name: 'Bad' }] }, { hosts: [hosts[0], hosts[0]] }, { hosts: [{ id: 'desk', name: '' }] }, { hosts: [] }, {}])('rejects malformed manifests without making host requests (%j)', async manifest => {
  fetcher.mockResolvedValue(json(manifest));
  render(<App />); await screen.findByRole('alert');
  expect(fetcher).toHaveBeenCalledTimes(1);
  expect(hostsSchema.safeParse(manifest).success).toBe(false);
});
it('only falls back to local mode on a manifest 404', async () => {
  fetcher.mockResolvedValue({ status: 404 } as Response);
  expect(await configuredHosts(new AbortController().signal)).toEqual([{ id: '', name: 'This host' }]);
  fetcher.mockResolvedValue({ status: 403 } as Response);
  await expect(configuredHosts(new AbortController().signal)).rejects.toThrow('403');
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
  const implementation = fetcher.getMockImplementation()!;
  fetcher.mockImplementation((url, options) => String(url).startsWith('/h/laptop/')
    ? new Promise((_, reject) => options!.signal!.addEventListener('abort', () => reject(options!.signal!.reason)))
    : implementation(url, options));
  render(<App />); await screen.findByText('Open projects on the sidebar.');
  const timeouts = timer.mock.calls.filter(([, delay]) => delay === 8_000);
  expect(timeouts).toHaveLength(2);
  await act(async () => { (timeouts[1][0] as () => void)(); });
  await screen.findByText(/Laptop: Asleep/);
  openMenu();
  expect(hostMenu('Desktop').getByRole('button', { name: project.name, exact: true })).toBeDefined();
});
it('preserves newer SSE status only on its own host during concurrent refresh', async () => {
  render(<App />); await screen.findByText('Open projects on the sidebar.'); await openLead('Laptop');
  const implementation = fetcher.getMockImplementation()!;
  let finish!: (value: Response) => void;
  fetcher.mockImplementation((url, options) => String(url) === '/h/laptop/api/sessions'
    ? new Promise(done => { finish = done; }) : implementation(url, options));
  fireEvent(window, new Event('focus'));
  Stream.opened.at(-1)!.emit('status', { ...session, agent: { ...session.agent, family: 'new-status' } });
  await screen.findByText('new-status');
  await act(async () => { finish(json({ api_version: 1, sessions: [session] })); });
  expect(screen.getByText('new-status')).toBeDefined();
});
it('reports malformed compatible host data without blocking the healthy host', async () => {
  failures.set('laptop', json({ api_version: 1, sessions: 'invalid', projects: [] }));
  render(<App />);
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toContain('Laptop: Invalid API response');
  await openLead('Desktop');
});
