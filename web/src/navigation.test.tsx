import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/preact';
import { App } from './App';
import { Navigation } from './Navigation';
import { projects, type Project, type Session } from './api';

const registered: Project[] = [
  { name: 'demo', state: 'waiting', workers: [
    { id: 'nav', skill: 'worker', lost: false, model: 'gpt-6.1' }, { id: 'review', skill: 'reviewer', lost: true },
    { id: 'audit', skill: 'security', lost: false }, { id: 'lookup', skill: 'research', lost: false },
    { id: 'plan', skill: 'custom-style', lost: false },
  ] },
  { name: 'offline', state: 'not running', workers: [] },
  { name: 'missing', state: 'missing', workers: [] },
];
let leads: Session[] = [{ id: 'demo:one', project: 'demo', agent: { family: 'codex', entry_skill: 'lead' }, waiting: true, worker_count: 5 }];
class Stream extends EventTarget {
  static urls: string[];
  constructor(url: string) { super(); Stream.urls.push(url); }
  close() {}
}
const openMenu = () => fireEvent.click(screen.getByRole('button', { name: 'Open menu' }));
const expandProject = (name: string) => {
  const button = screen.getByRole('button', { name, exact: true });
  if (button.getAttribute('aria-expanded') !== 'true') fireEvent.click(button);
};
beforeEach(() => {
  localStorage.clear(); window.history.replaceState(null, '', '/');
  Element.prototype.scrollIntoView = vi.fn(); Stream.urls = [];
  leads = [{ id: 'demo:one', project: 'demo', agent: { family: 'codex', entry_skill: 'lead' }, waiting: true, worker_count: 5 }];
  vi.stubGlobal('EventSource', Stream);
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve({ ok: true, json: async () =>
    url === '/api/projects' ? { api_version: 1, projects: registered } : { api_version: 1, sessions: leads },
  })));
});
it('shows the logo without auto-selecting a live project and opens the explorer menu', async () => {
  render(<App />);
  await screen.findByText('Open projects on the sidebar.');
  expect(screen.getByAltText('Niles logo')).toBeDefined();
  expect(Stream.urls).toEqual([]);
  expect(screen.queryByRole('navigation')).toBeNull();
  openMenu();
  const menu = screen.getByRole('navigation', { name: 'Projects' });
  expect(screen.queryByRole('button', { name: 'Stop demo' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Start offline' })).toBeNull();
  expect(menu.querySelectorAll('details')).toHaveLength(0);
  expandProject('demo');
  for (const text of ['demo', 'offline', 'waiting', 'nav', 'review', 'audit', 'lookup', 'plan', 'Lost']) expect(menu.textContent).toContain(text);
  expect([...menu.querySelectorAll('summary')].map(item => item.textContent)).toEqual(['custom-style', 'research', 'reviewer', 'security', 'worker']);
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
  await screen.findByRole('button', { name: 'demo', exact: true }); expandProject('demo');
  fireEvent.click(screen.getByRole('button', { name: 'Open demo lead chat' }));
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
  await screen.findByRole('button', { name: 'offline', exact: true }); expandProject('offline');
  fireEvent.click(screen.getByRole('button', { name: 'Open offline lead chat' }));
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
  expandProject('demo');
  fireEvent.click(screen.getByRole('button', { name: 'nav', exact: true }));
  fireEvent.click(screen.getByRole('button', { name: 'Open nav chat' }));
  await screen.findByRole('region', { name: 'Chat with nav' });
  expect(Stream.urls).toContain('/api/projects/demo/workers/nav/events');
  openMenu(); expandProject('demo');
  fireEvent.click(screen.getByRole('button', { name: 'Stop demo' }));
  expect(fetcher.mock.calls.some(([, options]) => options?.method === 'DELETE')).toBe(false);
  confirm.mockReturnValue(true);
  for (const [project, label, method, url] of [
    ['offline', 'Start offline', 'PUT', '/api/projects/offline/session'],
    ['demo', 'Stop demo', 'DELETE', '/api/projects/demo/session'],
    ['demo', 'Close review', 'DELETE', '/api/projects/demo/workers/review'],
  ]) {
    expandProject(project);
    if (label === 'Close review') fireEvent.click(screen.getByRole('button', { name: 'review', exact: true }));
    const before = fetcher.mock.calls.filter(([url]) => url === '/api/projects').length;
    fireEvent.click(screen.getByRole('button', { name: label }));
    await waitFor(() => expect(fetcher).toHaveBeenCalledWith(url, { method, cache: 'no-store' }));
    await waitFor(() => expect(fetcher.mock.calls.filter(([url]) => url === '/api/projects').length).toBeGreaterThan(before));
  }
  confirm.mockRestore();
});
it('expands only one project, supports collapsing it, and preserves expansion on refresh', async () => {
  render(<App />); await screen.findByText('Open projects on the sidebar.'); openMenu();
  expandProject('demo');
  expect(screen.getByRole('button', { name: 'demo', expanded: true })).toBeDefined();
  expect(screen.getByRole('button', { name: 'Stop demo' })).toBeDefined();
  expect(screen.getByRole('button', { name: 'nav', exact: true })).toBeDefined();
  expandProject('offline');
  expect(screen.getByRole('button', { name: 'demo', expanded: false })).toBeDefined();
  expect(screen.queryByRole('button', { name: 'Stop demo' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'nav', exact: true })).toBeNull();
  expect(screen.getByRole('button', { name: 'Start offline' })).toBeDefined();
  fireEvent(window, new Event('focus'));
  await waitFor(() => expect(fetch).toHaveBeenCalledTimes(4));
  expect(screen.getByRole('button', { name: 'offline', expanded: true })).toBeDefined();
  fireEvent.click(screen.getByRole('button', { name: 'offline', exact: true }));
  expect(screen.queryByRole('button', { name: 'Start offline' })).toBeNull();
  expect(screen.queryByRole('region', { name: /Chat with/ })).toBeNull();
});
it('preserves start conflict text and treats already gone as success', async () => {
  const { startProject, stopProject } = await import('./api');
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ status: 409, text: async () => '<b>No saved manifest</b>' }));
  await expect(startProject('demo')).rejects.toThrow('<b>No saved manifest</b>');
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ status: 404 }));
  await expect(stopProject('demo')).resolves.toBeUndefined();
});

it.each(['running', 'starting', 'waiting', 'not running', 'missing'] as const)('marks %s projects as live only when appropriate', state => {
  const { rerender } = render(<Navigation projects={[{ name: 'sample', state, workers: [] }]}
    openProject={vi.fn()} openWorker={vi.fn()} refresh={vi.fn()} help={vi.fn()} />);
  const button = screen.getByRole('button', { name: 'sample', expanded: false });
  const live = ['running', 'starting', 'waiting'].includes(state);
  expect(button.textContent?.includes('Live')).toBe(live);
  expect(screen.queryByRole('button', { name: 'sample', description: 'Live' }) !== null).toBe(live);
  rerender(<Navigation projects={[{ name: 'sample', state: live ? 'not running' : 'running', workers: [] }]}
    openProject={vi.fn()} openWorker={vi.fn()} refresh={vi.fn()} help={vi.fn()} />);
  expect(button.textContent?.includes('Live')).toBe(!live);
});

it('nests worker actions one at a time, labels skills and loss, and resets on project changes', async () => {
  render(<App />); await screen.findByText('Open projects on the sidebar.'); openMenu(); expandProject('demo');
  const nav = screen.getByRole('button', { name: 'nav', description: 'worker', expanded: false });
  const review = screen.getByRole('button', { name: 'review', description: 'reviewer · Lost', expanded: false });
  expect(screen.queryByRole('button', { name: /Open nav chat|Close nav|Close review/ })).toBeNull();
  fireEvent.click(nav);
  expect(screen.getByRole('button', { name: 'Open nav chat' })).toBeDefined();
  expect(screen.getByRole('button', { name: 'Close nav' })).toBeDefined();
  fireEvent.click(review);
  expect(nav.getAttribute('aria-expanded')).toBe('false');
  expect(screen.queryByRole('button', { name: 'Close nav' })).toBeNull();
  expect(screen.getByRole('button', { name: 'Close review' })).toBeDefined();
  fireEvent.click(review);
  expect(screen.queryByRole('button', { name: 'Close review' })).toBeNull();
  fireEvent.click(nav); expandProject('offline'); expandProject('demo');
  expect(screen.getByRole('button', { name: 'nav', expanded: false })).toBeDefined();
  expect(screen.queryByRole('button', { name: 'Open nav chat' })).toBeNull();
  expect(Stream.urls).toEqual([]);
});

it('uses one top toolbar for pane, model and connection in lead and worker chats', async () => {
  localStorage.setItem('niles.activeProject', 'demo');
  leads[0].agent = { ...leads[0].agent, model: 'opus-4.6' };
  const { container } = render(<App />);
  await screen.findByRole('region', { name: 'Chat with demo' });
  await screen.findByText('opus-4.6');
  const toolbar = container.querySelector('.toolbar')!;
  expect(toolbar.querySelector('h2')?.textContent).toBe('lead');
  expect(toolbar.querySelector('[role="status"]')?.textContent).toBe('catching up');
  expect(container.querySelector('.chat header')).toBeNull();
  expect(toolbar.lastElementChild?.getAttribute('alt')).toBe('Niles');
  openMenu(); expandProject('demo');
  fireEvent.click(screen.getByRole('button', { name: 'nav', exact: true }));
  fireEvent.click(screen.getByRole('button', { name: 'Open nav chat' }));
  await screen.findByRole('region', { name: 'Chat with nav' });
  await screen.findByText('gpt-6.1');
  expect(toolbar.querySelector('h2')?.textContent).toBe('worker · nav');
  expect(container.querySelectorAll('header')).toHaveLength(1);
});
