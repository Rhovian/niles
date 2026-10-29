import { beforeEach, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/preact';
import { App } from './App';
import { Chat } from './Chat';
import { Markdown } from './Markdown';
import DOMPurify from 'dompurify';
import { API_NAVIGATION_PATH } from './shell';
import { type Session, sessionsSchema, upsertTurn } from './api';

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
beforeEach(() => { window.history.replaceState(null, '', '/'); localStorage.clear();
  vi.stubGlobal('EventSource', Stream);
  Element.prototype.scrollIntoView = vi.fn();
});

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
it('upserts streaming revisions', () => {
  const turns = [{ id: 'server', role: 'user' as const, text: 'hello' }];
  expect(upsertTurn(turns, { ...turns[0], text: 'revised' })).toEqual([{ ...turns[0], text: 'revised' }]);
});
it('displays the active project lead’s foreground entry skill', async () => {
  localStorage.setItem('niles.activeProject', 'notes');
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve({ ok: true, json: async () => url === '/api/projects'
    ? { api_version: 1, projects: [{ name: 'notes', state: 'waiting', workers: [] }] }
    : { api_version: 1, sessions: [session] } })));
  render(<App />);
  await screen.findByRole('region', { name: 'Chat with notes' });
  expect(screen.getByText('codex')).toBeDefined();
});
it('blocks chat on API version mismatch', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => ({ api_version: 2, sessions: 'incompatible' }) }));
  render(<App />);
  await screen.findByText(/Update the web app/);
  expect(screen.getByRole('button', { name: 'Open menu' })).toBeDefined();
  expect(screen.queryByLabelText('Message agent')).toBeNull();
});
it('shows a tailnet banner on list network failure', async () => {
  vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Failed to fetch')));
  render(<App />);
  expect(await screen.findByRole('alert')).toHaveProperty('textContent', expect.stringContaining('tailnet'));
});
it('shows catching up until snapshot, reconnects, and fails closed on bad events', async () => {
  render(<Chat base={`/api/sessions/${encodeURIComponent(session.id)}`} id={session.id} title={session.project} subtitle={`${session.agent.family} · ${session.agent.entry_skill}`} onStatus={vi.fn()} />);
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
it.each(['turn', 'snapshot'] as const)('keeps sent text until its %s echo without matching older identical messages', async event => {
  let resolve!: (response: Response) => void;
  vi.stubGlobal('fetch', vi.fn<typeof globalThis.fetch>(() => new Promise(done => { resolve = done; })));
  render(<Chat base="/api/sessions/solo" id="solo" title="notes" subtitle="" />);
  const earlier = { id: 'earlier', role: 'user', text: 'hello' };
  Stream.latest.emit('snapshot', { turns: [earlier], truncated: false });
  fireEvent.input(screen.getByLabelText('Message agent'), { target: { value: 'hello' } });
  fireEvent.click(screen.getByText('Send'));
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
it('shows an unconfirmed send on non-200 without retrying', async () => {
  const fetch = vi.fn().mockResolvedValue({ status: 403 });
  vi.stubGlobal('fetch', fetch);
  render(<Chat base={`/api/sessions/${encodeURIComponent(session.id)}`} id={session.id} title={session.project} subtitle={`${session.agent.family} · ${session.agent.entry_skill}`} onStatus={vi.fn()} />);
  Stream.latest.emit('snapshot', { turns: [], truncated: false });
  fireEvent.input(screen.getByLabelText('Message agent'), { target: { value: 'hello' } });
  await waitFor(() => expect((screen.getByText('Send') as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByText('Send'));
  await screen.findByText(/Draft restored/);
  expect((screen.getByLabelText('Message agent') as HTMLTextAreaElement).value).toBe('hello');
  expect(screen.getByRole('alert').textContent).toContain('403');
  expect(fetch).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole('status', { name: 'Waiting for reply' })).toBeNull();
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
  render(<Chat base={`/api/sessions/${encodeURIComponent(session.id)}`} id={session.id} title={session.project} subtitle={`${session.agent.family} · ${session.agent.entry_skill}`} onStatus={vi.fn()} />);
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
  const { container } = render(<Chat base={`/api/sessions/${encodeURIComponent(session.id)}`} id={session.id} title={session.project} subtitle={`${session.agent.family} · ${session.agent.entry_skill}`} onStatus={vi.fn()} />);
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

it('dismisses the history notice after five seconds without live turns extending it', async () => {
  vi.useFakeTimers();
  try {
    render(<Chat base={`/api/sessions/${encodeURIComponent(session.id)}`} id={session.id} title={session.project} subtitle={`${session.agent.family} · ${session.agent.entry_skill}`} onStatus={vi.fn()} />);
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
  } finally { act(cleanup); vi.useRealTimers(); }
});

it('sends directly to the selected worker chat', async () => {
  const send = vi.fn().mockResolvedValue({ status: 200 }); vi.stubGlobal('fetch', send);
  render(<Chat base="/api/projects/demo/workers/nav" id="demo/nav" title="nav" subtitle="demo" />);
  Stream.latest.emit('snapshot', { turns: [], truncated: false });
  fireEvent.input(screen.getByLabelText('Message agent'), { target: { value: 'hello worker' } });
  fireEvent.click(screen.getByRole('button', { name: 'Send' }));
  await waitFor(() => expect(send).toHaveBeenCalledWith('/api/projects/demo/workers/nav/message', expect.objectContaining({ method: 'POST', body: JSON.stringify({ text: 'hello worker' }) })));
});

it.each(['/api/sessions/solo', '/api/projects/demo/workers/nav'])('animates waiting dots until a reply in %s', async base => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ status: 200 }));
  render(<Chat base={base} id="chat" title="chat" subtitle="" />);
  Stream.latest.emit('snapshot', { turns: [], truncated: false });
  expect(screen.queryByRole('status', { name: 'Waiting for reply' })).toBeNull();
  fireEvent.input(screen.getByLabelText('Message agent'), { target: { value: 'hello' } });
  fireEvent.click(screen.getByText('Send'));
  const indicator = await screen.findByRole('status', { name: 'Waiting for reply' });
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
  vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve({ ok: true, json: async () => url === '/api/projects'
    ? { api_version: 1, projects: [{ name: 'notes', state: 'waiting', workers: [] }] }
    : { api_version: 1, sessions: [session] } })));
  const previous = Stream.latest;
  render(<App />);
  await screen.findByRole('region', { name: 'Chat with notes' });
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
  render(<Chat base="/api/sessions/solo" id="solo" title="notes" subtitle="" />);
  Stream.latest.emit('snapshot', { turns: [], truncated: false });
  for (let n = 0; n < 2; n++) {
    fireEvent.input(screen.getByLabelText('Message agent'), { target: { value: 'same' } });
    fireEvent.click(screen.getByText('Send'));
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
  let complete!: (response: Response) => void;
  vi.stubGlobal('fetch', vi.fn<typeof globalThis.fetch>(() => new Promise(done => { complete = done; })));
  const chat = { base: '/api/sessions/solo', id: 'solo', title: 'notes', subtitle: '', working: false };
  const { rerender } = render(<Chat {...chat} />);
  Stream.latest.emit('snapshot', { turns: [], truncated: false });
  fireEvent.input(screen.getByLabelText('Message agent'), { target: { value: '**hi**\n\nsecond paragraph' } });
  fireEvent.click(screen.getByText('Send'));
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
