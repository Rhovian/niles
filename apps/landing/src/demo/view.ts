import { agentOf, project, rows, sameRow, type Agent, type Demo, type Panel, type Row } from './model';

// A styled run of terminal text; `c` names a class in the palette, `a` makes it a button for that action.
export type Seg = { t: string; c?: string; a?: string };
export type Line = Seg[];

const STATE_GLYPH = { working: '●', idle: '○', waiting: '⚠' } as const;
const PANEL_LABEL: Record<Panel, string> = {
  global: 'global', projects: 'projects', today: 'today', week: '7 days', month: '30 days', help: 'HELP',
};

const HELP: [string, string][] = [
  ['↑ ↓', 'Move'],
  ['→ ←', 'Expand or collapse a project'],
  ['Enter', 'Show the selected agent or panel; start a stopped project'],
  ['[ ]', 'Show the previous or next running project'],
  ['?', 'Show this help'],
  ['Esc', 'Show the selected item again'],
  ['c', 'Close the selected worker or project, after y/n'],
  ['Tab', 'Type to the shown agent; Esc comes back'],
];

export function tokens(n: number): string {
  return n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : `${Math.round(n / 1000)}k`;
}

/** Terminal columns a string takes: pictographs (emoji) two, joiners and variation selectors none. */
export function cols(s: string): number {
  let n = 0;
  for (const ch of s) n += /\p{Extended_Pictographic}/u.test(ch) ? 2 : /[\u200d\ufe00-\ufe0f]/u.test(ch) ? 0 : 1;
  return n;
}

// A word wider than the line (a pasted URL, a row of emoji) breaks at the width rather than running past it.
function chunks(word: string, width: number): string[] {
  const parts = [''];
  for (const ch of word) {
    if (parts[parts.length - 1] && cols(parts[parts.length - 1] + ch) > width) parts.push('');
    parts[parts.length - 1] += ch;
  }
  return parts;
}

export function wrap(text: string, width: number): string[] {
  const out: string[] = [];
  let line = '';
  for (const word of text.split(' ').flatMap((w) => chunks(w, width))) {
    if (line && cols(line) + 1 + cols(word) > width) {
      out.push(line);
      line = word;
    } else line = line ? `${line} ${word}` : word;
  }
  return [...out, line];
}

function agents(demo: Demo): Agent[] {
  return demo.projects.flatMap((p) => (p.lead ? [p.lead, ...p.workers] : []));
}

export function header(demo: Demo): Line {
  const all = agents(demo);
  const total = all.reduce((sum, a) => sum + a.tokens, 0);
  return [
    { t: 'NILES', c: 'brand' }, { t: `  ${demo.projects.length}`, c: 'b' }, { t: ' projects  ' },
    { t: `${all.length}`, c: 'ok' }, { t: ' agents  ' }, { t: tokens(total), c: 'b' }, { t: ' tok' },
  ];
}

function rowClass(demo: Demo, row: Row, index: number): string {
  const classes = [];
  if (index === demo.cursor) classes.push(demo.focus === 'explorer' ? 'sel' : 'sel-dim');
  if (sameRow(row, demo.shown)) classes.push('b');
  return classes.join(' ');
}

function stateSeg(a: Agent | null): Seg {
  return a ? { t: `${STATE_GLYPH[a.state]} `, c: a.state } : { t: '  ' };
}

export function explorer(demo: Demo): { line: Line; row: number | null }[] {
  const list = rows(demo);
  const out: { line: Line; row: number | null }[] = [];
  const heading = (t: string) => out.push({ line: [{ t, c: 'h' }], row: null });
  heading('PROJECTS');
  list.forEach((row, i) => {
    const cls = rowClass(demo, row, i);
    if (row.kind === 'project') {
      const p = project(demo, row.project);
      const last = demo.projects[demo.projects.length - 1] === p;
      const fold = p.workers.length ? (p.expanded ? '▾ ' : '▸ ') : '  ';
      out.push({ row: i, line: [{ t: last ? '└─ ' : '├─ ', c: 'dim' }, { t: fold, c: 'dim', a: p.workers.length ? 'toggle' : undefined }, stateSeg(p.lead),
        { t: p.name, c: cls }, ...(p.lead ? [{ t: ` ${tokens(p.lead.tokens)}`, c: 'dim' }] : [])] });
    } else if (row.kind === 'worker') {
      const workers = project(demo, row.project).workers;
      const w = workers.find((x) => x.id === row.worker) ?? null;
      const branch = workers[workers.length - 1] === w ? '│    └─ ' : '│    ├─ ';
      out.push({ row: i, line: [{ t: branch, c: 'dim' }, stateSeg(w), { t: row.worker, c: cls }, { t: ` ${tokens(w?.tokens ?? 0)}`, c: 'dim' }] });
    } else {
      if (row.panel === 'global') heading('CONFIG');
      if (row.panel === 'today') heading('TELEMETRY');
      const branch = row.panel === 'help' ? '' : ['projects', 'month'].includes(row.panel) ? '└─ ' : '├─ ';
      out.push({ row: i, line: [{ t: branch, c: 'dim' }, { t: PANEL_LABEL[row.panel], c: `${cls}${row.panel === 'help' ? ' h' : ''}` }] });
    }
  });
  return out;
}

export function hint(demo: Demo): Line {
  if (demo.confirm) {
    const target = demo.confirm.kind === 'worker' ? demo.confirm.worker : demo.confirm.kind === 'project' ? demo.confirm.project : '';
    return [{ t: `close ${target}? `, c: 'waiting' }, { t: 'y', c: 'b' }, { t: ' / ' }, { t: 'n', c: 'b' }];
  }
  return demo.focus === 'view'
    ? [{ t: 'typing to the shown agent · esc back', c: 'dim' }]
    : [{ t: '↵ open · → expand · c close · ? help · tab type', c: 'dim' }];
}

function transcript(a: Agent, width: number): Line[] {
  const out: Line[] = [];
  for (const e of a.transcript) {
    out.push([]);
    if (e.role === 'you') out.push([{ t: `❯ ${e.text}`, c: 'you' }]);
    else if (e.role === 'tool') out.push([{ t: '$ ', c: 'tool' }, { t: e.text, c: 'b' }]);
    else wrap(e.text, width - 2).forEach((t, i) => out.push([{ t: i ? '  ' : '• ', c: 'ok' }, { t }]));
  }
  return out;
}

function windowsLine(demo: Demo, row: Exclude<Row, { kind: 'panel' }>): Line {
  const p = project(demo, row.project);
  const names = p.lead ? ['lead', ...p.workers.map((w) => w.id)] : [];
  const current = row.kind === 'worker' ? row.worker : 'lead';
  return names.flatMap((n, i) => [{ t: `${i + 1}:${n}`, c: n === current ? 'tab' : 'dim' }, { t: ' ' }]);
}

function panel(demo: Demo, which: Panel, width: number): Line[] {
  const pad = (s: string, n: number) => s.padEnd(n);
  switch (which) {
    case 'help':
      return [[{ t: 'Keys', c: 'h' }], [], ...HELP.map(([k, v]) => [{ t: pad(k, 7), c: 'b' }, { t: v }])];
    case 'global':
      return [[{ t: 'Settings', c: 'h' }], [],
        [{ t: pad('theme', 15) }, { t: pad('tokyo-night', 14), c: 'b' }, { t: 'default', c: 'dim' }]];
    case 'projects':
      return [[{ t: 'Entry agents', c: 'h' }], [], ...demo.projects.filter((p) => p.lead)
        .map((p) => [{ t: pad(p.name, 10) }, { t: p.lead?.model ?? '', c: 'b' }])];
    default: {
      const scale = { today: 1, week: 6.2, month: 23.5 }[which];
      const running = demo.projects.filter((p) => p.lead);
      const sums = running.map((p) => [p.lead as Agent, ...p.workers].reduce((s, a) => s + a.tokens, 0) * scale);
      const max = Math.max(...sums, 1);
      const bar = Math.max(4, Math.min(24, width - 22));
      return [[{ t: `Tokens, ${PANEL_LABEL[which]}`, c: 'h' }], [], ...running.map((p, i) => {
        const fill = Math.round((sums[i] / max) * bar);
        return [{ t: pad(p.name, 9) }, { t: '█'.repeat(fill), c: 'brand' }, { t: '░'.repeat(bar - fill), c: 'dim' }, { t: ` ${tokens(sums[i])}`, c: 'b' }];
      })];
    }
  }
}

/** The view pane's lines for a given width in characters. */
export function view(demo: Demo, width: number): Line[] {
  const shown = demo.shown;
  if (shown.kind === 'panel') return panel(demo, shown.panel, width);
  const running = demo.projects.filter((p) => p.lead).map((p) => p.name);
  const tabs: Line = running.flatMap((n) => [{ t: ` ${n} `, c: n === shown.project ? 'tab' : 'dim' }, { t: '│', c: 'dim' }]);
  const a = agentOf(demo, shown);
  if (!a) return [tabs, [], [{ t: `${shown.project} is not running.` }], [{ t: 'Press Enter on it to start its lead.', c: 'dim' }]];
  const cursor = demo.focus === 'view' ? '▌' : '';
  return [tabs, windowsLine(demo, shown), ...transcript(a, width), [], [{ t: `❯ ${demo.draft}`, c: 'prompt' }, { t: cursor, c: 'brand' }]];
}
