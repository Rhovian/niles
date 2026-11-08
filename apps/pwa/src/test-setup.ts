import { afterEach, beforeEach, vi } from 'vitest';
import { act, cleanup } from '@testing-library/preact';
import { Stream } from './test-utils';

beforeEach(() => {
  if (typeof window === 'undefined') return;
  window.history.replaceState(null, '', '/'); localStorage.clear();
  Element.prototype.scrollIntoView = vi.fn(); Stream.opened = [];
  vi.stubGlobal('EventSource', Stream);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
});
// Preact 11 defers passive unmount cleanup; Testing Library cleanup does not use act.
// https://github.com/testing-library/preact-testing-library/blob/v3.2.4/src/pure.js
// https://github.com/preactjs/preact/blob/11.0.1/test-utils/src/index.js
afterEach(() => { act(cleanup); vi.useRealTimers(); vi.unstubAllGlobals(); });
