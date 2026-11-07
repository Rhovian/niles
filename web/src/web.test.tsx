import { createHash, webcrypto } from 'node:crypto';
import { Stream, deferred, serveLocal } from './test-utils';
import { expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/preact';
import { App } from './App';
import { Chat } from './Chat';
import { Markdown } from './Markdown';
import DOMPurify from 'dompurify';
import { API_NAVIGATION_PATH } from './shell';
import { type Session, sessionsSchema, upsertTurn } from './api';

const session: Session = { id: 'solo', project: 'notes', agent: { family: 'codex', entry_skill: 'solo' }, waiting: true, worker_count: 0 };

const emptySnapshot = { turns: [], truncated: false };
const renderChat = () => render(<Chat base="/api/sessions/solo" id={session.id} title={session.project} subtitle="" />);
function send(text: string) {
  fireEvent.input(screen.getByLabelText('Message agent'), { target: { value: text } });
  fireEvent.click(screen.getByText('Send'));
}

it('sanitizes every markdown render, memoizes it and opens links without an opener', () => {
  const sanitize = vi.spyOn(DOMPurify, 'sanitize');
  const { container, rerender } = render(<Markdown text={'**safe** <img src=x onerror=alert(1)><script>alert(1)</script><form><button>send</button></form>'} />);
  expect(container.querySelector('strong')?.textContent).toBe('safe');
  expect(container.querySelector('img,script,form,button')).toBeNull();
  rerender(<Markdown text={'[bad](javascript:alert(1)) <svg onload=alert(1)></svg><p id=app style="color:red" onclick="alert(1)">ok</p>'} />);
  expect(container.querySelector('[href],svg,[id],[style],[onclick]')).toBeNull();
  rerender(<Markdown text={'<a href="data:text/html,boom">bad</a><iframe src="/api"></iframe><object data="/api"></object>'} />);
  expect(container.querySelector('[href],iframe,object')).toBeNull();
  sanitize.mockClear();
  rerender(<Markdown text="[link](https://example.com)" />);
  rerender(<Markdown text="[link](https://example.com)" />);
  expect(sanitize).toHaveBeenCalledTimes(1);
  expect(container.querySelector('a')?.getAttribute('target')).toBe('_blank');
  expect(container.querySelector('a')?.getAttribute('rel')).toBe('noopener noreferrer');
});
it('rejects malformed session data at the boundary', () => {
  expect(() => sessionsSchema.parse({ api_version: 1, sessions: [{ ...session, worker_count: -1 }] })).toThrow();
});
it('upserts streaming revisions', () => {
  const turns = [{ id: 'server', role: 'user' as const, text: 'hello' }];
  expect(upsertTurn(turns, { ...turns[0], text: 'revised' })).toEqual([{ ...turns[0], text: 'revised' }]);
});
it('blocks chat on API version mismatch', async () => {
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(url === '/hosts.json' ? { status: 404 } : { ok: true, json: async () => ({ api_version: 2, sessions: 'incompatible' }) })));
  render(<App />);
  await screen.findByText(/Update the web app/);
  expect(screen.getByRole('button', { name: 'Open menu' })).toBeDefined();
  expect(screen.queryByLabelText('Message agent')).toBeNull();
});
it('shows catching up until snapshot, reconnects, and fails closed on bad events', async () => {
  renderChat();
  expect(screen.getByRole('status').textContent).toBe('catching up');
  Stream.latest.emit('snapshot', emptySnapshot);
  await waitFor(() => expect(screen.getByRole('status').textContent).toBe('live'));
  Stream.latest.onerror?.();
  await screen.findByText(/Reconnect Tailscale/);
  Stream.latest.onopen?.();
  await waitFor(() => expect(screen.getByRole('status').textContent).toBe('catching up'));
  Stream.latest.emit('snapshot', { turns: 'invalid' });
  await screen.findByText(/Invalid event/);
  expect(screen.getByRole('status').textContent).toBe('disconnected');
  expect(screen.queryByText(/Reconnect Tailscale/)).toBeNull();
  expect(Stream.latest.close).toHaveBeenCalled();
});
it.each(['turn', 'snapshot'] as const)('keeps sent text until its %s echo without matching older identical messages', async event => {
  const { promise, resolve } = deferred<Response>();
  vi.stubGlobal('fetch', vi.fn(() => promise));
  renderChat();
  const earlier = { id: 'earlier', role: 'user', text: 'hello' };
  Stream.latest.emit('snapshot', { turns: [earlier], truncated: false });
  send('hello');
  const element = (await screen.findByText('You · Sending…')).closest('article')!;
  const content = element.querySelector('.markdown');
  resolve(new Response(null, { status: 200 }));
  await screen.findByText('You · Sent');
  expect(screen.getAllByText('hello')).toHaveLength(2);
  const echo = { id: 'echo', role: 'user', text: 'hello' };
  await act(async () => { Stream.latest.emit(event, event === 'turn' ? echo : { turns: [earlier, echo], truncated: false }); });
  expect(screen.getByText('You · Sent').closest('article')).toBe(element);
  expect(element.querySelector('.markdown')).toBe(content);
  expect(screen.getAllByText('hello')).toHaveLength(2);
});

it('rejects status for a different session and closes on unmount', async () => {
  const onStatus = vi.fn();
  const { unmount } = render(<Chat base={`/api/sessions/${encodeURIComponent(session.id)}`} id={session.id} title={session.project} subtitle={`${session.agent.family} · ${session.agent.entry_skill}`} onStatus={onStatus} />);
  Stream.latest.emit('status', { ...session, id: 'other' });
  await screen.findByText(/Status session does not match/);
  expect(onStatus).not.toHaveBeenCalled();
  expect(Stream.latest.close).toHaveBeenCalledTimes(1);
  unmount();
  await waitFor(() => expect(Stream.latest.close).toHaveBeenCalledTimes(2));
});

it('excludes API navigations while keeping shell navigation available', () => {
  for (const path of ['/api', '/api/sessions', '/api/sessions/s/events']) expect(API_NAVIGATION_PATH.test(path)).toBe(true);
  for (const path of ['/', '/chat', '/apiary']) expect(API_NAVIGATION_PATH.test(path)).toBe(false);
});
it('surfaces a terminal stream failure and allows explicit reconnection', async () => {
  renderChat();
  Stream.latest.readyState = Stream.CLOSED;
  Stream.latest.onerror?.();
  await screen.findByText(/Session stream closed/);
  expect(screen.getByRole('status').textContent).toBe('disconnected');
  expect(screen.queryByText(/Reconnect Tailscale/)).toBeNull();
  const previous = Stream.latest;
  fireEvent.click(screen.getByText('Reconnect'));
  await waitFor(() => expect(Stream.latest).not.toBe(previous));
});
it.each([403, 'network'])('rejects premature submission and clears send failure %s on recovery', async failure => {
  const fetch = vi.fn().mockResolvedValue({ status: 200 });
  if (failure === 'network') fetch.mockRejectedValueOnce(new TypeError('Failed to fetch'));
  else fetch.mockResolvedValueOnce({ status: failure });
  vi.stubGlobal('fetch', fetch);
  const { container } = renderChat();
  fireEvent.input(screen.getByLabelText('Message agent'), { target: { value: 'hello' } });
  fireEvent.submit(container.querySelector('form')!);
  expect(fetch).not.toHaveBeenCalled();
  Stream.latest.emit('snapshot', emptySnapshot);
  await waitFor(() => expect(screen.getByRole('status').textContent).toBe('live'));
  fireEvent.submit(container.querySelector('form')!);
  await screen.findByText(/Draft restored/);
  expect((screen.getByLabelText('Message agent') as HTMLTextAreaElement).value).toBe('hello');
  expect(screen.getByRole('alert').textContent).toContain(failure === 'network' ? 'Reconnect Tailscale' : '403');
  expect(fetch).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole('status', { name: 'Waiting for reply' })).toBeNull();
  fireEvent.submit(container.querySelector('form')!);
  await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  expect(fetch).toHaveBeenCalledTimes(2);
});

it('dismisses the history notice after five seconds without live turns extending it', async () => {
  vi.useFakeTimers();
  renderChat();
  const snapshot = { turns: [{ id: 'first', role: 'agent', text: 'Recent message' }], truncated: true };
  await act(async () => { Stream.latest.emit('snapshot', snapshot); });
  expect(screen.getByText(/Showing the most recent 200 turns/)).toBeDefined();
  await act(async () => { vi.advanceTimersByTime(4_000); });
  await act(async () => { Stream.latest.emit('turn', { id: 'next', role: 'agent', text: 'New message' }); });
  await act(async () => { vi.advanceTimersByTime(999); });
  expect(screen.getByText(/Showing the most recent 200 turns/)).toBeDefined();
  await act(async () => { vi.advanceTimersByTime(1); });
  expect(screen.queryByText(/Showing the most recent 200 turns/)).toBeNull();
  expect(screen.getByText('Recent message')).toBeDefined();
  expect(screen.getByText('New message')).toBeDefined();
  await act(async () => { Stream.latest.emit('snapshot', snapshot); });
  expect(screen.queryByText(/Showing the most recent 200 turns/)).toBeNull();
});

it.each(['/api/sessions/solo', '/api/projects/demo/workers/nav'].flatMap(base => [null, { endpoint: 'https://fcm.googleapis.com/device' }, { endpoint: 'https://wns.example/unregistered' }].map(subscription => ({ base, subscription }))))('animates waiting dots and identifies the sending subscription ($base, $subscription)', async ({ base, subscription }) => {
  vi.stubGlobal('crypto', webcrypto);
  const getRegistration = vi.fn(async () => ({ pushManager: { getSubscription: async () => subscription } }));
  vi.stubGlobal('navigator', { serviceWorker: { getRegistration } });
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ status: 200 }));
  render(<Chat base={base} id="chat" title="chat" subtitle="" />);
  Stream.latest.emit('snapshot', emptySnapshot);
  expect(screen.queryByRole('status', { name: 'Waiting for reply' })).toBeNull();
  send('hello');
  const indicator = await screen.findByRole('status', { name: 'Waiting for reply' });
  await waitFor(() => expect(fetch).toHaveBeenCalledWith(`${base}/message`, expect.objectContaining({ method: 'POST', body: JSON.stringify({ text: 'hello', ...(subscription && base.includes('/api/sessions/') ? { sender_device: createHash('sha256').update(subscription.endpoint).digest('base64url') } : {}) }) })));
  expect(getRegistration).toHaveBeenCalledTimes(base.includes('/api/sessions/') ? 1 : 0);
  expect(indicator.children).toHaveLength(3);
  await waitFor(() => expect(screen.queryByText('You · Sending…')).toBeNull());
  Stream.latest.emit('turn', { id: 'echo', role: 'user', text: 'hello' });
  expect(screen.getByRole('status', { name: 'Waiting for reply' })).toBeDefined();
  Stream.latest.onerror?.();
  await waitFor(() => expect(screen.queryByRole('status', { name: 'Waiting for reply' })).toBeNull());
  Stream.latest.emit('snapshot', { turns: [{ id: 'echo', role: 'user', text: 'hello' }], truncated: false });
  await screen.findByRole('status', { name: 'Waiting for reply' });
  Stream.latest.emit('turn', { id: 'answer', role: 'agent', text: 'Reply' });
  await waitFor(() => expect(screen.queryByRole('status', { name: 'Waiting for reply' })).toBeNull());
});

it('keeps lead dots through agent prose until the server reports waiting again', async () => {
  localStorage.setItem('niles.activeProject', 'notes');
  serveLocal({ projects: [{ name: 'notes', state: 'waiting', workers: [] }], sessions: [session] });
  const previous = Stream.latest;
  render(<App />);
  await screen.findByRole('region', { name: 'Chat with notes' });
  expect(screen.getByText('codex')).toBeDefined();
  await waitFor(() => expect(Stream.latest).not.toBe(previous));
  Stream.latest.emit('snapshot', { turns: [{ id: 'user', role: 'user', text: 'hello' }], truncated: false });
  await waitFor(() => expect(screen.getByRole('status').textContent).toBe('live'));
  expect(screen.queryByRole('status', { name: 'Waiting for reply' })).toBeNull();
  vi.mocked(Element.prototype.scrollIntoView).mockClear();
  Stream.latest.emit('status', { ...session, waiting: false });
  await screen.findByRole('status', { name: 'Waiting for reply' });
  expect(Element.prototype.scrollIntoView).not.toHaveBeenCalled();
  Stream.latest.emit('turn', { id: 'prose', role: 'agent', text: 'Still working…' });
  await screen.findByText('Still working…');
  expect(screen.getByRole('status', { name: 'Waiting for reply' })).toBeDefined();
  Stream.latest.emit('status', { ...session, waiting: true });
  await waitFor(() => expect(screen.queryByRole('status', { name: 'Waiting for reply' })).toBeNull());
});

it('hands off identical pending sends one echo at a time, even before POST completion', async () => {
  const complete: ((response: Response) => void)[] = [];
  vi.stubGlobal('fetch', vi.fn<typeof globalThis.fetch>(() => new Promise(done => { complete.push(done); })));
  renderChat();
  Stream.latest.emit('snapshot', emptySnapshot);
  for (let n = 0; n < 2; n++) {
    send('same');
  }
  await waitFor(() => expect(screen.getAllByText('same')).toHaveLength(2));
  Stream.latest.emit('turn', { id: 'quick', role: 'agent', text: 'Quick reply' });
  await waitFor(() => expect(screen.queryByRole('status', { name: 'Waiting for reply' })).toBeNull());
  const echo = { id: 'first', role: 'user', text: 'same' };
  Stream.latest.emit('turn', echo);
  await waitFor(() => expect(screen.getAllByText('You · Sending…')).toHaveLength(1));
  expect(screen.getAllByText('same')).toHaveLength(2);
  Stream.latest.emit('snapshot', { turns: [echo], truncated: false });
  expect(screen.getAllByText('same')).toHaveLength(2);
  Stream.latest.emit('turn', { ...echo, id: 'second' });
  await waitFor(() => expect(screen.queryByText('You · Sending…')).toBeNull());
  await act(async () => { complete.forEach(done => done(new Response(null, { status: 200 }))); });
  expect(screen.getAllByText('same')).toHaveLength(2);
});

it('preserves the sending message node, Markdown content and position through early replies and echo', async () => {
  const { promise, resolve: complete } = deferred<Response>();
  vi.stubGlobal('fetch', vi.fn(() => promise));
  const chat = { base: '/api/sessions/solo', id: 'solo', title: 'notes', subtitle: '', working: false };
  const { rerender } = render(<Chat {...chat} />);
  Stream.latest.emit('snapshot', emptySnapshot);
  send('**hi**\n\nsecond paragraph');
  const element = (await screen.findByText('You · Sending…')).closest('article')!;
  const content = element.querySelector('.markdown');
  expect(content?.querySelector('strong')?.textContent).toBe('hi');
  await act(async () => { complete(new Response(null, { status: 200 })); });
  rerender(<Chat {...chat} working />);
  await act(async () => { Stream.latest.emit('turn', { id: 'reply', role: 'agent', text: 'Working' }); });
  await act(async () => { Stream.latest.emit('turn', { id: 'echo', role: 'user', text: '**hi**\n\nsecond paragraph' }); });
  expect(screen.getByText('You · Sent').closest('article')).toBe(element);
  expect(element.querySelector('.markdown')).toBe(content);
  expect(element.parentElement?.querySelector('article')).toBe(element);
});
