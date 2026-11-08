import type { Demo } from './model';
import { explorer, header, hint, view, type Line, type Seg } from './view';

const ESCAPES: Record<string, string> = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' };
const escape = (s: string) => s.replace(/[&<>"]/g, (ch) => ESCAPES[ch]);

function seg(s: Seg): string {
  if (s.a) return `<button type="button" class="${s.c ?? ''}" data-act="${escape(s.a)}">${escape(s.t)}</button>`;
  return s.c ? `<span class="${s.c}">${escape(s.t)}</span>` : escape(s.t);
}

export function line(l: Line, attrs = ''): string {
  const body = l.map(seg).join('');
  return `<div class="tl"${attrs}>${body || ' '}</div>`;
}

/** The terminal's panes as HTML, for the build and the live demo alike. */
export function panes(demo: Demo, viewWidth: number) {
  return {
    head: line(header(demo)),
    explorer: explorer(demo).map((e) => line(e.line, e.row === null ? '' : ` data-row="${e.row}"`)).join(''),
    view: view(demo, viewWidth).map((l) => line(l)).join(''),
    foot: line(hint(demo)),
  };
}
