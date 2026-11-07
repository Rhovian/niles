import { beforeEach, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/preact';
import { Telemetry } from './Telemetry';
import { App } from './App';
import { telemetrySchema } from './telemetry-data';

const hosts = [{ id: 'desk', name: 'Desktop' }, { id: 'laptop', name: 'Laptop' }];
const collected_at = new Date().toISOString();
const bucket = new Date(Math.floor(Date.now() / 900_000) * 900_000).toISOString();
const snapshot = { api_version: 1, collected_at, sessions: [{
  project: 'demo', id: 'same-id', entry: true, family: 'pi', model: 'test-model',
  created_at: collected_at, closed_at: null, buckets: { [bucket]: 42 },
}] };
const json = (body: unknown) => ({ ok: true, status: 200, json: async () => body }) as Response;
let failures: Map<string, Error | Response>;
let fetcher: ReturnType<typeof vi.fn<typeof fetch>>;
const region = (name: string) => within(screen.getByRole('region', { name: `${name} telemetry` }));
beforeEach(() => {
  localStorage.clear(); failures = new Map();
  fetcher = vi.fn(async input => {
    const url = String(input);
    if (url === '/hosts.json') return json({ hosts });
    const failure = failures.get(url);
    if (failure instanceof Error) throw failure;
    return failure ?? json(snapshot);
  });
  vi.stubGlobal('fetch', fetcher);
});
it('fetches machines concurrently, isolates colliding sessions and opens from the PWA menu', async () => {
  const original = fetcher.getMockImplementation()!;
  let finish!: (response: Response) => void;
  fetcher.mockImplementation((url, options) => String(url) === '/h/laptop/api/telemetry'
    ? new Promise(done => { finish = done; }) : String(url).endsWith('/api/sessions')
      ? Promise.resolve(json({ api_version: 1, sessions: [] })) : String(url).endsWith('/api/projects')
        ? Promise.resolve(json({ api_version: 1, projects: [] })) : original(url, options));
  render(<App />);
  fireEvent.click(screen.getByRole('button', { name: 'Open menu' }));
  fireEvent.click(screen.getByRole('button', { name: 'Telemetry', exact: true }));
  await screen.findByRole('region', { name: 'Desktop telemetry' });
  await region('Desktop').findByText('42 tokens');
  expect(region('Laptop').getByText('Connecting…')).toBeDefined();
  expect(fetcher).toHaveBeenCalledWith('/h/laptop/api/telemetry', expect.objectContaining({ cache: 'no-store' }));
  await act(async () => { finish(json(snapshot)); });
  await screen.findByRole('region', { name: 'Laptop telemetry' });
  await region('Laptop').findByText('42 tokens');
  expect(region('Desktop').getByText('demo / same-id')).toBeDefined();
  expect(region('Laptop').getByText('demo / same-id')).toBeDefined();
  expect(screen.getByText('84 tokens in cached snapshots')).toBeDefined();
});
it('retains last good data and collection time across remount, failure and recovery', async () => {
  const app = render(<Telemetry />);
  await screen.findByRole('region', { name: 'Laptop telemetry' });
  await region('Laptop').findByText('42 tokens');
  app.unmount(); failures.set('/h/laptop/api/telemetry', { ok: false, status: 502 } as Response);
  render(<Telemetry />);
  await screen.findByRole('region', { name: 'Laptop telemetry' });
  await region('Laptop').findByText('Asleep or unreachable');
  expect(region('Laptop').getByText('42 tokens')).toBeDefined();
  expect(region('Laptop').getByText(new Date(collected_at).toLocaleString(), { selector: 'time' }).getAttribute('datetime')).toBe(collected_at);
  failures.set('/h/laptop/api/telemetry', { ok: false, status: 500 } as Response);
  fireEvent.click(screen.getByRole('button', { name: 'Refresh telemetry' }));
  expect((await region('Laptop').findByRole('alert')).textContent).toContain('500');
  expect(region('Laptop').getByText('42 tokens')).toBeDefined();
  expect(JSON.parse(localStorage.getItem('niles.telemetry.v1:laptop')!)).toEqual(snapshot);
  failures.clear(); fireEvent.click(screen.getByRole('button', { name: 'Refresh telemetry' }));
  await region('Laptop').findByText('Connected');
  expect(region('Laptop').queryByRole('alert')).toBeNull();
});
it('keeps an HTTP 500 host visible without cached data', async () => {
  failures.set('/h/laptop/api/telemetry', { ok: false, status: 500 } as Response);
  render(<Telemetry />);
  await screen.findByRole('alert');
  expect(region('Laptop').getByRole('alert').textContent).toContain('500');
  expect(region('Laptop').getByText('No cached usage.')).toBeDefined();
  expect(region('Desktop').getByText('42 tokens')).toBeDefined();
});
it.each([{ ...snapshot, api_version: 99 }, { ...snapshot, sessions: 'invalid' }])('shows incompatible or malformed responses as failed (%j)', async data => {
  failures.set('/h/laptop/api/telemetry', json(data));
  render(<Telemetry />); await screen.findByRole('alert');
  expect(region('Laptop').getByRole('alert').textContent).toContain('Failed:');
  expect(localStorage.getItem('niles.telemetry.v1:laptop')).toBeNull();
});
it('times out a hung host without blocking another host', async () => {
  const original = fetcher.getMockImplementation()!;
  fetcher.mockImplementation((url, options) => String(url) === '/h/laptop/api/telemetry'
    ? new Promise((_, reject) => options!.signal!.addEventListener('abort', () => reject(options!.signal!.reason)))
    : original(url, options));
  const timeout = new AbortController();
  vi.spyOn(AbortSignal, 'timeout').mockReturnValue(timeout.signal);
  render(<Telemetry />); await screen.findByRole('region', { name: 'Desktop telemetry' }); await region('Desktop').findByText('42 tokens');
  await act(async () => timeout.abort(new DOMException('timed out', 'TimeoutError')));
  await screen.findByRole('region', { name: 'Laptop telemetry' });
  await region('Laptop').findByText('Asleep or unreachable');
});
it('ignores a cancelled refresh and uses the newest response', async () => {
  const original = fetcher.getMockImplementation()!;
  let finish!: (response: Response) => void;
  fetcher.mockImplementationOnce(original).mockImplementationOnce(() => new Promise(done => { finish = done; }));
  render(<Telemetry />); await screen.findByRole('region', { name: 'Desktop telemetry' }); await region('Laptop').findByText('42 tokens');
  fireEvent.click(screen.getByRole('button', { name: 'Refresh telemetry' }));
  await screen.findByRole('region', { name: 'Desktop telemetry' });
  await region('Desktop').findByText('42 tokens');
  await act(async () => { finish(json({ ...snapshot, sessions: [] })); });
  expect(region('Desktop').getByText('42 tokens')).toBeDefined();
  expect(JSON.parse(localStorage.getItem('niles.telemetry.v1:desk')!)).toEqual(snapshot);
});
it('filters time buckets and rejects unaligned buckets at the boundary', async () => {
  const now = Math.floor(Date.now() / 900_000) * 900_000 + 300_000;
  vi.spyOn(Date, 'now').mockReturnValue(now);
  const old = new Date(now - 2 * 86_400_000 - 300_000).toISOString();
  const edge = new Date(now - 86_400_000 - 300_000).toISOString();
  const data = { ...snapshot, sessions: [
    { ...snapshot.sessions[0], buckets: { [bucket]: 42, [edge]: 2 } },
    { ...snapshot.sessions[0], id: 'old-session', created_at: old, closed_at: old, buckets: { [old]: 100 } },
  ] };
  fetcher.mockImplementation(async url => String(url) === '/hosts.json' ? json({ hosts: [hosts[0]] }) : json(data));
  render(<Telemetry />); await screen.findByRole('region', { name: 'Desktop telemetry' }); await region('Desktop').findByText('144 tokens');
  fireEvent.input(screen.getByLabelText('Time range'), { target: { value: '1' } });
  await region('Desktop').findByText('44 tokens');
  expect(region('Desktop').queryByText('demo / old-session')).toBeNull();
  expect(telemetrySchema.safeParse({ ...snapshot, sessions: [{ ...snapshot.sessions[0], buckets: { '2026-10-01T00:01:00Z': 1 } }] }).success).toBe(false);
});
it('uses the local backend only when the manifest is absent', async () => {
  fetcher.mockImplementation(async url => String(url) === '/hosts.json' ? { status: 404 } as Response : json(snapshot));
  render(<Telemetry />); await screen.findByRole('region', { name: 'This host telemetry' }); await region('This host').findByText('42 tokens');
  expect(fetcher).toHaveBeenCalledWith('/api/telemetry', expect.objectContaining({ cache: 'no-store' }));
  await waitFor(() => expect(fetcher).toHaveBeenCalledTimes(2));
});

it('replaces incompatible and malformed cached snapshots from healthy hosts', async () => {
  localStorage.setItem('niles.telemetry.v1:desk', JSON.stringify({ ...snapshot, api_version: 99 }));
  localStorage.setItem('niles.telemetry.v1:laptop', '{bad json');
  render(<Telemetry />);
  await screen.findByRole('region', { name: 'Desktop telemetry' });
  await region('Desktop').findByText('Connected');
  await region('Laptop').findByText('Connected');
  for (const host of hosts) expect(JSON.parse(localStorage.getItem(`niles.telemetry.v1:${host.id}`)!)).toEqual(snapshot);
});
it('preserves row identity for reused worker IDs when snapshots reorder them', async () => {
  const earlier = { ...snapshot.sessions[0], entry: false, created_at: new Date(Date.now() - 3_600_000).toISOString() };
  const later = { ...snapshot.sessions[0], entry: false };
  let sessions = [earlier, later];
  fetcher.mockImplementation(async url => String(url) === '/hosts.json' ? json({ hosts: [hosts[0]] }) : json({ ...snapshot, sessions }));
  render(<Telemetry />); await screen.findByRole('region', { name: 'Desktop telemetry' });
  await region('Desktop').findByText('Connected');
  const before = region('Desktop').getAllByRole('row');
  sessions = [later, earlier];
  fireEvent.click(screen.getByRole('button', { name: 'Refresh telemetry' }));
  await region('Desktop').findByText('Connected');
  await waitFor(() => expect(JSON.parse(localStorage.getItem('niles.telemetry.v1:desk')!).sessions[0].created_at).toBe(later.created_at));
  const after = region('Desktop').getAllByRole('row');
  expect(after[1]).toBe(before[2]);
  expect(after[2]).toBe(before[1]);
});

it('keeps fresh usage visible and names storage failure if browser quota is exhausted', async () => {
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new DOMException('full', 'QuotaExceededError'); });
  render(<Telemetry />); await screen.findByRole('region', { name: 'Desktop telemetry' });
  expect((await region('Desktop').findByRole('alert')).textContent).toContain('Cannot cache telemetry');
  expect(region('Desktop').getByText('42 tokens')).toBeDefined();
});
