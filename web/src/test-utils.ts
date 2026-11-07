import { vi } from 'vitest';
import { fireEvent, screen } from '@testing-library/preact';
import type { Project, Session } from './api';

export class Stream extends EventTarget {
  static opened: Stream[] = [];
  static get latest() { return Stream.opened[Stream.opened.length - 1]; }
  static CLOSED = 2;
  static get urls() { return Stream.opened.map(stream => stream.url); }
  readyState = 1;
  onopen?: () => void;
  onerror?: () => void;
  close = vi.fn();
  constructor(readonly url: string) { super(); Stream.opened.push(this); }
  emit(name: string, data: unknown) { this.dispatchEvent(new MessageEvent(name, { data: JSON.stringify(data) })); }
}

export const json = (data: unknown) => ({ ok: true, status: 200, json: async () => data }) as Response;
export const openMenu = () => fireEvent.click(screen.getByRole('button', { name: 'Open menu' }));
export function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
export function serveLocal({ projects, sessions }: { projects: Project[]; sessions: Session[] | (() => Promise<Response>) }) {
  const fetcher = vi.fn<typeof fetch>(input => String(input) === '/hosts.json' ? Promise.resolve(new Response(null, { status: 404 }))
    : String(input) === '/api/projects' ? Promise.resolve(json({ api_version: 1, projects }))
    : typeof sessions === 'function' ? sessions() : Promise.resolve(json({ api_version: 1, sessions })));
  vi.stubGlobal('fetch', fetcher);
  return fetcher;
}
