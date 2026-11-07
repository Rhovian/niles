import { beforeEach, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/preact';
import { Telemetry } from './Telemetry';
import { App } from './App';
import { BUCKET_MS, telemetrySchema } from './telemetry-data';
import { type Host, hostPrefix } from './hosts';

const hosts = [{ id: 'desk', name: 'Desktop' }, { id: 'laptop', name: 'Laptop' }];
const collected_at = new Date().toISOString();
const bucket = new Date(Math.floor(Date.now() / BUCKET_MS) * BUCKET_MS).toISOString();
const snapshot = { api_version: 1, collected_at, sessions: [{
  project: 'demo', id: 'same-id', entry: true, family: 'pi', model: 'test-model',
  created_at: collected_at, closed_at: null, buckets: { [bucket]: 42 },
}] };
const json = (body: unknown) => ({ ok: true, status: 200, json: async () => body }) as Response;
type Override = Error | Response | Promise<Response> | ((options?: RequestInit) => Response | Promise<Response>);
let failures: Map<string, Override>;
let fetcher: ReturnType<typeof vi.fn<typeof fetch>>;
const findRegion = async (name: string) => within(await screen.findByRole('region', { name: `${name} telemetry` }));
function serve(configured: Host[] | null, body: unknown | (() => unknown)) {
  failures.set('/hosts.json', configured ? json({ hosts: configured }) : { status: 404 } as Response);
  for (const host of configured ?? [{ id: '', name: 'This host' }]) {
    failures.set(`${hostPrefix(host)}/api/telemetry`, () => json(typeof body === 'function' ? body() : body));
  }
}
beforeEach(() => {
  localStorage.clear(); failures = new Map();
  fetcher = vi.fn(async (input, options) => {
    const url = String(input), override = failures.get(url);
    if (override instanceof Error) throw override;
    if (override) return typeof override === 'function' ? override(options) : override;
    if (url === '/hosts.json') return json({ hosts });
    if (url.endsWith('/api/projects')) return json({ api_version: 1, projects: [] });
    if (url.endsWith('/api/sessions')) return json({ api_version: 1, sessions: [] });
    return json(snapshot);
  });
  vi.stubGlobal('fetch', fetcher);
});
it('fetches machines concurrently, isolates colliding sessions and opens from the PWA menu', async () => {
  let finish!: (response: Response) => void;
  failures.set('/h/laptop/api/telemetry', new Promise(done => { finish = done; }));
  render(<App />);
  fireEvent.click(screen.getByRole('button', { name: 'Open menu' }));
  fireEvent.click(screen.getByRole('button', { name: 'Telemetry', exact: true }));
  const desktop = await findRegion('Desktop'), laptop = await findRegion('Laptop');
  await desktop.findByText('42 tokens');
  expect(laptop.getByText('Connecting…')).toBeDefined();
  expect(fetcher).toHaveBeenCalledWith('/h/laptop/api/telemetry', expect.objectContaining({ cache: 'no-store' }));
  await act(async () => { finish(json(snapshot)); });
  await laptop.findByText('42 tokens');
  expect(desktop.getByText('demo / same-id')).toBeDefined();
  expect(laptop.getByText('demo / same-id')).toBeDefined();
  expect(screen.getByText('84 tokens in cached snapshots')).toBeDefined();
});
it('retains last good data and collection time across remount, failure and recovery', async () => {
  const app = render(<Telemetry />);
  await (await findRegion('Laptop')).findByText('42 tokens');
  app.unmount(); failures.set('/h/laptop/api/telemetry', { ok: false, status: 502 } as Response);
  render(<Telemetry />);
  const laptop = await findRegion('Laptop');
  await laptop.findByText('Asleep or unreachable');
  expect(laptop.getByText('42 tokens')).toBeDefined();
  expect(laptop.getByText(new Date(collected_at).toLocaleString(), { selector: 'time' }).getAttribute('datetime')).toBe(collected_at);
  failures.set('/h/laptop/api/telemetry', { ok: false, status: 500 } as Response);
  fireEvent.click(screen.getByRole('button', { name: 'Refresh telemetry' }));
  expect((await laptop.findByRole('alert')).textContent).toContain('500');
  expect(laptop.getByText('42 tokens')).toBeDefined();
  expect(JSON.parse(localStorage.getItem('niles.telemetry.v1:laptop')!)).toEqual(snapshot);
  failures.clear(); fireEvent.click(screen.getByRole('button', { name: 'Refresh telemetry' }));
  await laptop.findByText('Connected');
  expect(laptop.queryByRole('alert')).toBeNull();
});
it('keeps an HTTP 500 host visible without cached data', async () => {
  failures.set('/h/laptop/api/telemetry', { ok: false, status: 500 } as Response);
  render(<Telemetry />);
  expect((await (await findRegion('Laptop')).findByRole('alert')).textContent).toContain('500');
  expect((await findRegion('Laptop')).getByText('No cached usage.')).toBeDefined();
  expect((await findRegion('Desktop')).getByText('42 tokens')).toBeDefined();
});
it.each([
  [{ api_version: 99, sessions: 'future schema' }, 'Update the web app. API version 99'],
  [{ ...snapshot, sessions: 'invalid' }, 'Failed:'],
])('distinguishes incompatible versions from malformed responses (%j)', async (data, message) => {
  failures.set('/h/laptop/api/telemetry', json(data));
  render(<Telemetry />);
  expect((await (await findRegion('Laptop')).findByRole('alert')).textContent).toContain(message);
  expect(localStorage.getItem('niles.telemetry.v1:laptop')).toBeNull();
});
it('times out a hung host without blocking another host', async () => {
  failures.set('/h/laptop/api/telemetry', options => new Promise((_, reject) =>
    options!.signal!.addEventListener('abort', () => reject(options!.signal!.reason))));
  const timeout = new AbortController();
  vi.spyOn(AbortSignal, 'timeout').mockReturnValue(timeout.signal);
  render(<Telemetry />); await (await findRegion('Desktop')).findByText('42 tokens');
  await act(async () => timeout.abort(new DOMException('timed out', 'TimeoutError')));
  await (await findRegion('Laptop')).findByText('Asleep or unreachable');
});
it('ignores a cancelled refresh and uses the newest response', async () => {
  let finish!: (response: Response) => void;
  failures.set('/h/desk/api/telemetry', new Promise(done => { finish = done; }));
  render(<Telemetry />); await (await findRegion('Laptop')).findByText('42 tokens');
  failures.delete('/h/desk/api/telemetry');
  fireEvent.click(screen.getByRole('button', { name: 'Refresh telemetry' }));
  const desktop = await findRegion('Desktop');
  await desktop.findByText('42 tokens');
  await act(async () => { finish(json({ ...snapshot, sessions: [] })); });
  expect(desktop.getByText('42 tokens')).toBeDefined();
  expect(JSON.parse(localStorage.getItem('niles.telemetry.v1:desk')!)).toEqual(snapshot);
});
it('filters time buckets and rejects unaligned buckets at the boundary', async () => {
  const now = Math.floor(Date.now() / BUCKET_MS) * BUCKET_MS + 300_000;
  vi.spyOn(Date, 'now').mockReturnValue(now);
  const old = new Date(now - 2 * 86_400_000 - 300_000).toISOString();
  const edge = new Date(now - 86_400_000 - 300_000).toISOString();
  serve([hosts[0]], { ...snapshot, sessions: [
    { ...snapshot.sessions[0], buckets: { [bucket]: 42, [edge]: 2 } },
    { ...snapshot.sessions[0], id: 'old-session', created_at: old, closed_at: old, buckets: { [old]: 100 } },
  ] });
  render(<Telemetry />); const desktop = await findRegion('Desktop'); await desktop.findByText('144 tokens');
  fireEvent.input(screen.getByLabelText('Time range'), { target: { value: '1' } });
  await desktop.findByText('44 tokens');
  expect(desktop.queryByText('demo / old-session')).toBeNull();
  expect(telemetrySchema.safeParse({ ...snapshot, sessions: [{ ...snapshot.sessions[0], buckets: { '2026-10-01T00:01:00Z': 1 } }] }).success).toBe(false);
});
it('uses the local backend only when the manifest is absent', async () => {
  serve(null, snapshot);
  render(<Telemetry />); await (await findRegion('This host')).findByText('42 tokens');
  expect(fetcher).toHaveBeenCalledWith('/api/telemetry', expect.objectContaining({ cache: 'no-store' }));
  await waitFor(() => expect(fetcher).toHaveBeenCalledTimes(2));
});
it('replaces incompatible and malformed cached snapshots from healthy hosts', async () => {
  localStorage.setItem('niles.telemetry.v1:desk', JSON.stringify({ ...snapshot, api_version: 99 }));
  localStorage.setItem('niles.telemetry.v1:laptop', '{bad json');
  render(<Telemetry />);
  await (await findRegion('Desktop')).findByText('Connected');
  await (await findRegion('Laptop')).findByText('Connected');
  for (const host of hosts) expect(JSON.parse(localStorage.getItem(`niles.telemetry.v1:${host.id}`)!)).toEqual(snapshot);
});
it('preserves row identity for reused worker IDs when snapshots reorder them', async () => {
  const earlier = { ...snapshot.sessions[0], entry: false, created_at: new Date(Date.now() - 3_600_000).toISOString() };
  const later = { ...snapshot.sessions[0], entry: false };
  let sessions = [earlier, later];
  serve([hosts[0]], () => ({ ...snapshot, sessions }));
  render(<Telemetry />); const desktop = await findRegion('Desktop'); await desktop.findByText('Connected');
  const before = desktop.getAllByRole('row');
  sessions = [later, earlier];
  fireEvent.click(screen.getByRole('button', { name: 'Refresh telemetry' }));
  await waitFor(() => expect(JSON.parse(localStorage.getItem('niles.telemetry.v1:desk')!).sessions[0].created_at).toBe(later.created_at));
  const after = desktop.getAllByRole('row');
  expect(after[1]).toBe(before[2]);
  expect(after[2]).toBe(before[1]);
});
it('keeps fresh usage visible and names storage failure if browser quota is exhausted', async () => {
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new DOMException('full', 'QuotaExceededError'); });
  render(<Telemetry />); const desktop = await findRegion('Desktop');
  expect((await desktop.findByRole('alert')).textContent).toContain('Cannot cache telemetry');
  expect(desktop.getByText('42 tokens')).toBeDefined();
});
