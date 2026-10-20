import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/preact';
import { App } from './App';

const session = { id: 'solo', project: 'notes', agent: { family: 'codex', entry_skill: 'solo' }, waiting: false, worker_count: 0 };
const response = (sessions: unknown[]) => ({ ok: true, json: async () => ({ api_version: 1, sessions }) });
beforeEach(() => { Element.prototype.scrollIntoView = vi.fn(); });
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
it('refreshes on focus and the 15s timer without overlapping, and reports ended sessions', async () => {
  let tick!: () => void;
  const interval = vi.spyOn(globalThis, 'setInterval');
  let resolve!: (result: ReturnType<typeof response>) => void;
  const fetch = vi.fn(() => new Promise(done => { resolve = done; }));
  vi.stubGlobal('fetch', fetch);
  class Stream extends EventTarget { close() {} }
  vi.stubGlobal('EventSource', Stream);
  render(<App />);
  tick = interval.mock.calls.find(([, delay]) => delay === 15_000)![0] as () => void;
  fireEvent(window, new Event('focus'));
  tick();
  expect(fetch).toHaveBeenCalledTimes(1);
  expect(interval).toHaveBeenCalledWith(expect.any(Function), 15_000);
  resolve(response([session]));
  await screen.findByText('notes');
  fireEvent.click(screen.getByRole('button'));
  tick();
  expect(fetch).toHaveBeenCalledTimes(2);
  resolve(response([]));
  await screen.findByText('This session has ended.');
  fireEvent(window, new Event('focus'));
  expect(fetch).toHaveBeenCalledTimes(3);
  resolve(response([]));
});
it('does not overwrite an SSE status with an older in-flight snapshot', async () => {
  let stream!: EventTarget;
  class Stream extends EventTarget { constructor() { super(); stream = this; } close() {} }
  vi.stubGlobal('EventSource', Stream);
  let resolve!: (result: ReturnType<typeof response>) => void;
  vi.stubGlobal('fetch', vi.fn().mockResolvedValueOnce(response([session])).mockImplementation(() => new Promise(done => { resolve = done; })));
  render(<App />);
  await screen.findByText('notes');
  fireEvent.click(screen.getByRole('button'));
  await waitFor(() => expect(stream).toBeDefined());
  fireEvent(window, new Event('focus'));
  stream.dispatchEvent(new MessageEvent('status', { data: JSON.stringify({ ...session, waiting: true }) }));
  await screen.findByText(/⚠ Waiting/);
  await act(async () => { resolve(response([session])); });
  expect(screen.getByText(/⚠ Waiting/)).toBeDefined();
});
it.each([401, 403, 500])('reports HTTP %s without claiming a tailnet failure', async status => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status }));
  render(<App />);
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toContain(String(status));
  expect(alert.textContent).not.toContain('tailnet');
});
