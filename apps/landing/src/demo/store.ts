import { initialDemo, type Demo } from './model';

// One staged session for the page: the terminal and the phone are two views on it, as the real
// PWA is a second view on a live session.
let state: Demo = initialDemo();
const listeners = new Set<(demo: Demo) => void>();

export const store = {
  get: (): Demo => state,
  set(next: Demo): void {
    state = next;
    for (const listener of listeners) listener(state);
  },
  subscribe(listener: (demo: Demo) => void): void {
    listeners.add(listener);
  },
};
