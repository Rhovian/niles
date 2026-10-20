import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/preact';
import { App } from './App';
import { Chat } from './Chat';
import { Markdown } from './Markdown';
import DOMPurify from 'dompurify';
import { API_NAVIGATION_PATH } from './shell';
import { type Session, sessionsSchema, sortedSessions, upsertTurn } from './api';

const session: Session = { id: 'solo', project: 'notes', agent: { family: 'codex', entry_skill: 'solo' }, waiting: true, worker_count: 0 };
class Stream extends EventTarget {
  static latest: Stream;
  static CLOSED = 2;
  readyState = 1;
  onopen?: () => void;
  onerror?: () => void;
  close = vi.fn();
  constructor() { super(); Stream.latest = this; }
  emit(name: string, data: unknown) { this.dispatchEvent(new MessageEvent(name, { data: JSON.stringify(data) })); }
}
beforeEach(() => {
  vi.stubGlobal('EventSource', Stream);
  Element.prototype.scrollIntoView = vi.fn();
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

it('sanitizes every markdown render, including markup, URLs, forms and attributes', () => {
  const { container, rerender } = render(<Markdown text={'**safe** <img src=x onerror=alert(1)><script>alert(1)</script><form><button>send</button></form>'} />);
  expect(container.querySelector('strong')?.textContent).toBe('safe');
  expect(container.querySelector('img,script,form,button')).toBeNull();
  rerender(<Markdown text={'[bad](javascript:alert(1)) <svg onload=alert(1)></svg><p id=app style="color:red" onclick="alert(1)">ok</p>'} />);
  expect(container.querySelector('[href],svg,[id],[style],[onclick]')).toBeNull();
  rerender(<Markdown text={'<a href="data:text/html,boom">bad</a><iframe src="/api"></iframe><object data="/api"></object>'} />);
  expect(container.querySelector('[href],iframe,object')).toBeNull();
});
it('rejects malformed session data at the boundary', () => {
  expect(() => sessionsSchema.parse({ api_version: 1, sessions: [{ ...session, worker_count: -1 }] })).toThrow();
});
it('upserts streaming revisions and sorts projects ascending within waiting groups', () => {
  const turns = [{ id: 'server', role: 'user' as const, text: 'hello' }];
  expect(upsertTurn(turns, { ...turns[0], text: 'revised' })).toEqual([{ ...turns[0], text: 'revised' }]);
  expect(sortedSessions([{ ...session, project: 'z' }, { ...session, project: 'a' }]).map(item => item.project)).toEqual(['a', 'z']);
});
it('pins waiting sessions and displays the foreground entry skill', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => ({ api_version: 1, sessions: [{ ...session, id: 'busy', waiting: false }, session] }) }));
  render(<App />);
  await screen.findAllByText('notes');
  const buttons = screen.getAllByRole('button');
  expect(buttons[0].textContent).toContain('⚠ Waiting');
  expect(buttons[0].textContent).toContain('solo');
});
it('blocks chat on API version mismatch', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => ({ api_version: 2, sessions: 'incompatible' }) }));
  render(<App />);
  await screen.findByText(/Update the web app/);
  expect(screen.queryByRole('button')).toBeNull();
  expect(screen.queryByLabelText('Message agent')).toBeNull();
});
it('shows a tailnet banner on list network failure', async () => {
  vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Failed to fetch')));
  render(<App />);
  expect(await screen.findByRole('alert')).toHaveProperty('textContent', expect.stringContaining('tailnet'));
});
it('shows catching up until snapshot, reconnects, and fails closed on bad events', async () => {
  render(<Chat session={session} onStatus={vi.fn()} />);
  expect(screen.getByRole('status').textContent).toBe('catching up');
  Stream.latest.emit('snapshot', { turns: [], truncated: false });
  await waitFor(() => expect(screen.getByRole('status').textContent).toBe('live'));
  Stream.latest.onerror?.();
  await screen.findByText(/Check that you’re connected/);
  Stream.latest.onopen?.();
  await waitFor(() => expect(screen.getByRole('status').textContent).toBe('catching up'));
  Stream.latest.emit('snapshot', { turns: 'invalid' });
  await screen.findByText(/Invalid event/);
  expect(screen.getByRole('status').textContent).toBe('disconnected');
  expect(screen.queryByText(/Check that you’re connected/)).toBeNull();
  expect(Stream.latest.close).toHaveBeenCalled();
});
it('drops optimistic send on 200 without requiring transcript IDs', async () => {
  let resolve!: (response: Response) => void;
  vi.stubGlobal('fetch', vi.fn<typeof globalThis.fetch>(() => new Promise(done => { resolve = done; })));
  render(<Chat session={session} onStatus={vi.fn()} />);
  Stream.latest.emit('snapshot', { turns: [], truncated: false });
  fireEvent.input(screen.getByLabelText('Message agent'), { target: { value: 'hello' } });
  await waitFor(() => expect((screen.getByText('Send') as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByText('Send'));
  await screen.findByText('You · Sending…');
  resolve(new Response(null, { status: 200 }));
  await waitFor(() => expect(screen.queryByText('You · Sending…')).toBeNull());
  Stream.latest.emit('snapshot', { turns: [{ id: 'server', role: 'user', text: 'hello' }], truncated: false });
  expect(await screen.findAllByText('hello')).toHaveLength(1);
});
it('shows an unconfirmed send on non-200 without retrying', async () => {
  const fetch = vi.fn().mockResolvedValue({ status: 403 });
  vi.stubGlobal('fetch', fetch);
  render(<Chat session={session} onStatus={vi.fn()} />);
  Stream.latest.emit('snapshot', { turns: [], truncated: false });
  fireEvent.input(screen.getByLabelText('Message agent'), { target: { value: 'hello' } });
  await waitFor(() => expect((screen.getByText('Send') as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByText('Send'));
  await screen.findByText(/Draft restored/);
  expect((screen.getByLabelText('Message agent') as HTMLTextAreaElement).value).toBe('hello');
  expect(screen.getByRole('alert').textContent).toContain('403');
  expect(fetch).toHaveBeenCalledTimes(1);
});

it('rejects status for a different session and closes on unmount', async () => {
  const onStatus = vi.fn();
  const { unmount } = render(<Chat session={session} onStatus={onStatus} />);
  Stream.latest.emit('status', { ...session, id: 'other' });
  await screen.findByText(/Status session does not match/);
  expect(onStatus).not.toHaveBeenCalled();
  expect(Stream.latest.close).toHaveBeenCalledTimes(1);
  unmount();
  await waitFor(() => expect(Stream.latest.close).toHaveBeenCalledTimes(2));
});

it('memoizes sanitized markdown and opens links without an opener', () => {
  const sanitize = vi.spyOn(DOMPurify, 'sanitize');
  const { container, rerender } = render(<Markdown text="[link](https://example.com)" />);
  rerender(<Markdown text="[link](https://example.com)" />);
  expect(sanitize).toHaveBeenCalledTimes(1);
  expect(container.querySelector('a')?.getAttribute('target')).toBe('_blank');
  expect(container.querySelector('a')?.getAttribute('rel')).toBe('noopener noreferrer');
});
it('excludes API navigations while keeping shell navigation available', () => {
  for (const path of ['/api', '/api/sessions', '/api/sessions/s/events']) expect(API_NAVIGATION_PATH.test(path)).toBe(true);
  for (const path of ['/', '/chat', '/apiary']) expect(API_NAVIGATION_PATH.test(path)).toBe(false);
});
it('surfaces a terminal stream failure and allows explicit reconnection', async () => {
  render(<Chat session={session} onStatus={vi.fn()} />);
  Stream.latest.readyState = Stream.CLOSED;
  Stream.latest.onerror?.();
  await screen.findByText(/Session stream closed/);
  expect(screen.getByRole('status').textContent).toBe('disconnected');
  expect(screen.queryByText(/Check that you’re connected/)).toBeNull();
  const previous = Stream.latest;
  fireEvent.click(screen.getByText('Reconnect'));
  await waitFor(() => expect(Stream.latest).not.toBe(previous));
});
it('rejects form submission before a snapshot and clears stale send errors on recovery', async () => {
  const fetch = vi.fn().mockResolvedValueOnce({ status: 403 }).mockResolvedValue({ status: 200 });
  vi.stubGlobal('fetch', fetch);
  const { container } = render(<Chat session={session} onStatus={vi.fn()} />);
  fireEvent.input(screen.getByLabelText('Message agent'), { target: { value: 'hello' } });
  fireEvent.submit(container.querySelector('form')!);
  expect(fetch).not.toHaveBeenCalled();
  Stream.latest.emit('snapshot', { turns: [], truncated: false });
  await waitFor(() => expect(screen.getByRole('status').textContent).toBe('live'));
  fireEvent.submit(container.querySelector('form')!);
  await screen.findByText(/Draft restored/);
  fireEvent.submit(container.querySelector('form')!);
  await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
});
