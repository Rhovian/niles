import { Stream, json, openMenu, deferred, serveLocal } from './test-utils';
import { expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/preact';
import { App } from './App';

const session = { id: 'solo', project: 'notes', agent: { family: 'codex', entry_skill: 'solo' }, waiting: false, worker_count: 0 };
const response = (sessions: unknown[]) => json({ api_version: 1, sessions });
const projects = [{ name: 'notes', state: 'running' as const, workers: [] }];
async function openLead() {
  openMenu();
  fireEvent.click(await screen.findByRole('button', { name: 'notes', exact: true }));
  fireEvent.click(screen.getByRole('button', { name: 'Open notes lead chat' }));
}
it('refreshes on focus and the 15s timer without overlapping, and reports ended sessions', async () => {
  const interval = vi.spyOn(globalThis, 'setInterval');
  let pending = deferred<Response>();
  const fetch = serveLocal({ projects, sessions: () => (pending = deferred<Response>()).promise });
  render(<App />);
  await waitFor(() => expect(fetch).toHaveBeenCalledTimes(3));
  const tick = interval.mock.calls.find(([, delay]) => delay === 15_000)![0] as () => void;
  fireEvent(window, new Event('focus'));
  tick();
  expect(fetch).toHaveBeenCalledTimes(3);
  expect(interval).toHaveBeenCalledWith(expect.any(Function), 15_000);
  pending.resolve(response([session]));
  await openLead();
  tick();
  expect(fetch).toHaveBeenCalledTimes(5);
  pending.resolve(response([]));
  await screen.findByText(/This project’s lead is not running/);
  fireEvent(window, new Event('focus'));
  expect(fetch).toHaveBeenCalledTimes(7);
  pending.resolve(response([]));
});
it('does not overwrite an SSE status with an older in-flight snapshot', async () => {
  const pending = deferred<Response>();
  let calls = 0;
  serveLocal({ projects, sessions: () => ++calls === 1 ? Promise.resolve(response([session])) : pending.promise });
  render(<App />);
  await openLead();
  await waitFor(() => expect(Stream.latest).toBeDefined());
  fireEvent(window, new Event('focus'));
  Stream.latest.dispatchEvent(new MessageEvent('status', { data: JSON.stringify({ ...session, agent: { family: 'claude', entry_skill: 'lead' } }) }));
  await screen.findByText('claude');
  await act(async () => { pending.resolve(response([session])); });
  expect(screen.getByText('claude')).toBeDefined();
});
it.each([401, 403, 500, 'network'] as const)('reports list failure %s with the appropriate tailnet guidance', async failure => {
  vi.stubGlobal('fetch', failure === 'network' ? vi.fn().mockRejectedValue(new TypeError('Failed to fetch')) : vi.fn().mockResolvedValue({ ok: false, status: failure }));
  render(<App />);
  const alert = await screen.findByRole('alert');
  if (failure !== 'network') expect(alert.textContent).toContain(String(failure));
  expect(alert.textContent?.includes('tailnet')).toBe(failure === 'network');
});
