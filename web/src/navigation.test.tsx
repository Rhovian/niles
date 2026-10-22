import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/preact';
import { App } from './App';
import { projects, type Project } from './api';

const registered: Project[] = [
  { name: 'demo', state: 'waiting', workers: [
    { id: 'nav', role: 'worker', lost: false }, { id: 'review', role: 'reviewer', lost: true },
    { id: 'audit', role: 'security', lost: false }, { id: 'lookup', role: 'research', lost: false },
    { id: 'plan', role: 'design', lost: false },
  ] },
  { name: 'offline', state: 'not running', workers: [] },
  { name: 'missing', state: 'missing', workers: [] },
];
let leads = [{ id: 'demo:one', project: 'demo', agent: { family: 'codex', entry_skill: 'lead' }, waiting: true, worker_count: 5 }];
class Stream extends EventTarget {
  static urls: string[];
  constructor(url: string) { super(); Stream.urls.push(url); }
  close() {}
}
const openMenu = () => fireEvent.click(screen.getByRole('button', { name: 'Open menu' }));
beforeEach(() => {
  localStorage.clear(); window.history.replaceState(null, '', '/');
  Element.prototype.scrollIntoView = vi.fn(); Stream.urls = [];
  leads = [{ id: 'demo:one', project: 'demo', agent: { family: 'codex', entry_skill: 'lead' }, waiting: true, worker_count: 5 }];
  vi.stubGlobal('EventSource', Stream);
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve({ ok: true, json: async () =>
    url === '/api/projects' ? { api_version: 1, projects: registered } : { api_version: 1, sessions: leads },
  })));
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
it('shows the logo without auto-selecting a live project and opens the explorer menu', async () => {
  render(<App />);
  await screen.findByText('Open projects on the sidebar.');
  expect(screen.getByAltText('Niles logo')).toBeDefined();
  expect(Stream.urls).toEqual([]);
  expect(screen.queryByRole('navigation')).toBeNull();
  openMenu();
  const menu = screen.getByRole('navigation', { name: 'Projects' });
  for (const text of ['demo', 'offline', 'waiting', 'not running', 'nav', 'review', 'audit', 'lookup', 'plan', 'Lost']) expect(menu.textContent).toContain(text);
  expect([...menu.querySelectorAll('summary')].map(item => item.textContent)).toEqual(['worker', 'reviewer', 'security', 'research', 'design']);
  expect(menu.querySelectorAll('details')).toHaveLength(5);
  expect(screen.queryByText('Config')).toBeNull(); expect(screen.queryByText('Telemetry')).toBeNull();
  expect(screen.getByRole('button', { name: 'nav', exact: true })).toBeDefined();
  expect(screen.queryByRole('button', { name: 'Start missing' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Help' }));
  expect(screen.getByRole('heading', { name: 'Help' })).toBeDefined();
  expect(screen.queryByRole('navigation')).toBeNull();
});
it('opens and remembers the project, including after its lead restarts', async () => {
  const app = render(<App />); openMenu();
  fireEvent.click(await screen.findByRole('button', { name: /demo.*waiting/ }));
  await screen.findByRole('region', { name: 'Chat with demo' });
  expect(localStorage.getItem('niles.activeProject')).toBe('demo');
  expect(screen.queryByRole('navigation')).toBeNull();
  expect(Stream.urls).toContain('/api/sessions/demo%3Aone/events');
  leads = [{ ...leads[0], id: 'demo:two' }];
  fireEvent(window, new Event('focus'));
  await waitFor(() => expect(Stream.urls).toContain('/api/sessions/demo%3Atwo/events'));
  app.unmount(); window.history.replaceState(null, '', '/'); render(<App />);
  await screen.findByRole('region', { name: 'Chat with demo' });
});
it('remembers an inactive project', async () => {
  const app = render(<App />); openMenu();
  fireEvent.click(await screen.findByRole('button', { name: /offline.*not running/ }));
  await screen.findByText(/This project’s lead is not running/);
  expect(localStorage.getItem('niles.activeProject')).toBe('offline');
  app.unmount(); render(<App />);
  await screen.findByText(/This project’s lead is not running/);
  expect(Stream.urls).toEqual([]);
});
it('opens notification session links and remembers their project', async () => {
  window.history.replaceState(null, '', '/#/sessions/demo%3Aone'); render(<App />);
  await screen.findByRole('region', { name: 'Chat with demo' });
  await waitFor(() => expect(localStorage.getItem('niles.activeProject')).toBe('demo'));
});
it('does not open a different remembered project for an expired notification', async () => {
  localStorage.setItem('niles.activeProject', 'demo');
  window.history.replaceState(null, '', '/#/sessions/expired'); render(<App />);
  await screen.findByText(/This project’s lead is not running/);
  expect(Stream.urls).toEqual([]);
});
it('closes the menu with Escape or its backdrop', async () => {
  render(<App />); await screen.findByText('Open projects on the sidebar.');
  openMenu(); fireEvent.keyDown(screen.getByRole('navigation'), { key: 'Escape' });
  expect(screen.queryByRole('navigation')).toBeNull();
  openMenu(); fireEvent.click(screen.getByRole('button', { name: 'Dismiss menu' }));
  expect(screen.queryByRole('navigation')).toBeNull();
});
it('checks project API versions before parsing and rejects malformed compatible data', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => ({ api_version: 99, projects: 'changed' }) }));
  expect(await projects()).toEqual({ api_version: 99, projects: [] });
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => ({ api_version: 1, projects: [{ name: 'bad', state: 'unknown', workers: [] }] }) }));
  await expect(projects()).rejects.toThrow();
});

it('opens worker transcripts and exposes confirmed lifecycle actions with refetch', async () => {
  const fetcher = vi.mocked(fetch);
  const initial = fetcher.getMockImplementation()!;
  fetcher.mockImplementation((url, options) => options?.method
    ? Promise.resolve({ status: 200 } as Response) : initial(url, options));
  const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
  render(<App />); await screen.findByText('Open projects on the sidebar.'); openMenu();
  fireEvent.click(screen.getByRole('button', { name: 'nav', exact: true }));
  await screen.findByRole('region', { name: 'Chat with nav' });
  expect(Stream.urls).toContain('/api/projects/demo/workers/nav/events');
  openMenu();
  fireEvent.click(screen.getByRole('button', { name: 'Stop demo' }));
  expect(fetcher.mock.calls.some(([, options]) => options?.method === 'DELETE')).toBe(false);
  confirm.mockReturnValue(true);
  for (const [label, method, url] of [
    ['Start offline', 'PUT', '/api/projects/offline/session'],
    ['Stop demo', 'DELETE', '/api/projects/demo/session'],
    ['Close review', 'DELETE', '/api/projects/demo/workers/review'],
  ]) {
    const before = fetcher.mock.calls.filter(([url]) => url === '/api/projects').length;
    fireEvent.click(screen.getByRole('button', { name: label }));
    await waitFor(() => expect(fetcher).toHaveBeenCalledWith(url, { method, cache: 'no-store' }));
    await waitFor(() => expect(fetcher.mock.calls.filter(([url]) => url === '/api/projects').length).toBeGreaterThan(before));
  }
  confirm.mockRestore();
});
it('preserves start conflict text and treats already gone as success', async () => {
  const { startProject, stopProject } = await import('./api');
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ status: 409, text: async () => '<b>No saved manifest</b>' }));
  await expect(startProject('demo')).rejects.toThrow('<b>No saved manifest</b>');
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ status: 404 }));
  await expect(stopProject('demo')).resolves.toBeUndefined();
});
