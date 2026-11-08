// The lockup in assets/niles-lockup-amber.svg is drawn on character grids already: the butler in
// 6px pixels, two to a 6x12 character cell, and the wordmark in ANSI Shadow cells with thin
// double-line shadows. These read it back as text, so the page's logo follows the SVG.

type Rect = { x: number; y: number; w: number; h: number };

function group(svg: string, name: 'mark' | 'word'): Rect[] {
  const body = svg.match(new RegExp(`<g class="${name}"[^>]*>([\\s\\S]*?)</g>`))?.[1];
  if (!body) throw new Error(`lockup SVG has no "${name}" group`);
  return [...body.matchAll(/<rect x="([\d.]+)" y="([\d.]+)" width="([\d.]+)" height="([\d.]+)"/g)]
    .map(([, x, y, w, h]) => ({ x: +x, y: +y, w: +w, h: +h }));
}

function grid(cells: Map<string, string>): string[] {
  const keys = [...cells.keys()].map((k) => k.split(',').map(Number));
  const cols = Math.max(...keys.map(([c]) => c)) + 1;
  const rows = Math.max(...keys.map(([, r]) => r)) + 1;
  return Array.from({ length: rows }, (_, r) =>
    Array.from({ length: cols }, (_, c) => cells.get(`${c},${r}`) ?? ' ').join('').trimEnd());
}

/** The butler as half blocks: one character per pixel column, two pixel rows per character row. */
export function butler(svg: string): string[] {
  const rects = group(svg, 'mark');
  const pixel = Math.min(...rects.map((r) => r.w));
  const x0 = Math.min(...rects.map((r) => r.x));
  const y0 = Math.min(...rects.map((r) => r.y));
  const on = new Set<string>();
  for (const r of rects) {
    for (let dy = 0; dy < Math.round(r.h / pixel); dy++) on.add(`${Math.round((r.x - x0) / pixel)},${Math.round((r.y - y0) / pixel) + dy}`);
  }
  const cells = new Map<string, string>();
  for (const key of on) {
    const [c, p] = key.split(',').map(Number);
    const top = p - (p % 2);
    const glyph = on.has(`${c},${top}`) ? (on.has(`${c},${top + 1}`) ? '█' : '▀') : '▄';
    cells.set(`${c},${top / 2}`, glyph);
  }
  return grid(cells);
}

// A shadow cell's arms (left, right, up, down) name its box-drawing character.
const SHADOW: Record<string, string> = { LR: '═', DU: '║', DL: '╗', DR: '╔', LU: '╝', RU: '╚' };

/** The wordmark in ANSI Shadow: full blocks, and box-drawing shadows from the thin lines. */
export function wordmark(svg: string): string[] {
  const rects = group(svg, 'word');
  const cw = Math.max(...rects.map((r) => r.w));
  const ch = Math.max(...rects.map((r) => r.h));
  const x0 = Math.min(...rects.map((r) => r.x));
  const y0 = Math.min(...rects.map((r) => r.y));
  const cells = new Map<string, string>();
  const arms = new Map<string, Set<string>>();
  const near = (a: number, b: number) => Math.abs(a - b) < 0.6;
  for (const r of rects) {
    const c = Math.floor((r.x + 0.01 - x0) / cw);
    const row = Math.floor((r.y + 0.01 - y0) / ch);
    if (near(r.w, cw) && near(r.h, ch)) { cells.set(`${c},${row}`, '█'); continue; }
    const left = x0 + c * cw, top = y0 + row * ch;
    const set = arms.get(`${c},${row}`) ?? new Set<string>();
    if (r.h < r.w) { if (near(r.x, left)) set.add('L'); if (near(r.x + r.w, left + cw)) set.add('R'); }
    else { if (near(r.y, top)) set.add('U'); if (near(r.y + r.h, top + ch)) set.add('D'); }
    arms.set(`${c},${row}`, set);
  }
  for (const [key, set] of arms) {
    if (cells.has(key)) continue;
    const glyph = SHADOW[[...set].sort().join('')];
    if (!glyph) throw new Error(`wordmark cell ${key} has arms ${[...set].join('')}`);
    cells.set(key, glyph);
  }
  return grid(cells);
}
