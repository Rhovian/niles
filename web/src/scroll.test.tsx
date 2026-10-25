import { afterEach, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/preact';
import { Chat } from './Chat';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
it('follows viewport/content changes, pauses on scroll up, and resumes at bottom', async () => {
  let resize!: () => void;
  const disconnect = vi.fn();
  vi.stubGlobal('ResizeObserver', class { constructor(callback: () => void) { resize = callback; } observe() {} disconnect = disconnect; });
  class Stream extends EventTarget { static latest: Stream; constructor() { super(); Stream.latest = this; } close() {} }
  vi.stubGlobal('EventSource', Stream);
  render(<Chat base="/api/sessions/s" id="s" title="chat" subtitle="" />);
  const transcript = screen.getByLabelText('Transcript');
  let height = 1_000;
  Object.defineProperties(transcript, { scrollHeight: { get: () => height }, clientHeight: { value: 200 } });
  await act(async () => { resize(); });
  expect(transcript.scrollTop).toBe(1_000);
  transcript.scrollTop = 500; fireEvent.scroll(transcript);
  height = 1_200;
  await act(async () => { resize(); });
  expect(transcript.scrollTop).toBe(500);
  await act(async () => { Stream.latest.dispatchEvent(new MessageEvent('snapshot', { data: JSON.stringify({ turns: [{ id: 't', role: 'agent', text: 'Updated' }], truncated: false }) })); });
  expect(transcript.scrollTop).toBe(500);
  transcript.scrollTop = 1_000; fireEvent.scroll(transcript);
  height = 1_400;
  await act(async () => { resize(); });
  expect(transcript.scrollTop).toBe(1_400);
  cleanup(); expect(disconnect).toHaveBeenCalled();
});
