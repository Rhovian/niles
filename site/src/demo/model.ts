// A staged copy of the Niles home view: fictional projects, canned replies, nothing runs.

export type AgentState = 'working' | 'idle' | 'waiting';
export type Entry = { role: 'you' | 'agent' | 'tool'; text: string };

export type Agent = {
  id: string;
  model: string;
  state: AgentState;
  tokens: number;
  transcript: Entry[];
};

export type Project = {
  name: string;
  lead: Agent | null;
  workers: Agent[];
  expanded: boolean;
};

export type Panel = 'global' | 'projects' | 'today' | 'week' | 'month' | 'help';

export type Row =
  | { kind: 'project'; project: string }
  | { kind: 'worker'; project: string; worker: string }
  | { kind: 'panel'; panel: Panel };

export type Demo = {
  projects: Project[];
  cursor: number;
  shown: Row;
  focus: 'explorer' | 'view';
  draft: string;
  confirm: Row | null;
  spawned: number;
};

const PANELS: Panel[] = ['global', 'projects', 'today', 'week', 'month', 'help'];

// Workers spawn on the next family in turn, so a request shows the mixing the page describes.
const SPAWN_MODELS = ['codex:gpt-6.1-sol:medium', 'claude:opus:high', 'pi:z-ai/glm-5.3-flash:high'];

export const SPAWN_PATTERN = /\b(yes|go|spawn|review)\b/i;

function agent(id: string, model: string, state: AgentState, tokens: number, transcript: Entry[]): Agent {
  return { id, model, state, tokens, transcript };
}

export function initialDemo(): Demo {
  const billing: Project = {
    name: 'billing',
    expanded: true,
    lead: agent('lead', 'claude:opus:medium', 'working', 318_000, [
      { role: 'you', text: 'Webhook retries double-charge on timeout. Issue #412.' },
      { role: 'tool', text: 'gh issue view 412' },
      { role: 'agent', text: 'One worker can fix the retry key; a reviewer from another family checks the diff. Spawned w412 on codex:gpt-6.1-sol:medium.' },
    ]),
    workers: [
      agent('w412', 'codex:gpt-6.1-sol:medium', 'working', 212_000, [
        { role: 'tool', text: 'cat src/webhooks/retry.rs' },
        { role: 'agent', text: 'The retry reuses the attempt id, so the provider sees a new charge. Keying it on the event id; adding a test.' },
      ]),
    ],
  };
  const docs: Project = {
    name: 'docs',
    expanded: false,
    lead: agent('lead', 'codex:gpt-6.1-sol:medium', 'waiting', 96_000, [
      { role: 'agent', text: 'The API reference is rebuilt and the broken links are fixed. Merge PR #88, or should a reviewer look first?' },
    ]),
    workers: [],
  };
  return {
    projects: [billing, docs, { name: 'mobile', expanded: false, lead: null, workers: [] }, { name: 'infra', expanded: false, lead: null, workers: [] }],
    cursor: 0,
    shown: { kind: 'project', project: 'billing' },
    focus: 'explorer',
    draft: '',
    confirm: null,
    spawned: 0,
  };
}

export function rows(demo: Demo): Row[] {
  const out: Row[] = [];
  for (const p of demo.projects) {
    out.push({ kind: 'project', project: p.name });
    if (p.expanded) for (const w of p.workers) out.push({ kind: 'worker', project: p.name, worker: w.id });
  }
  for (const panel of PANELS) out.push({ kind: 'panel', panel });
  return out;
}

export function sameRow(a: Row, b: Row): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

export function project(demo: Demo, name: string): Project {
  const p = demo.projects.find((q) => q.name === name);
  if (!p) throw new Error(`no project ${name}`);
  return p;
}

/** The agent a row shows in the view pane: a project's lead or a worker. */
export function agentOf(demo: Demo, row: Row): Agent | null {
  if (row.kind === 'panel') return null;
  const p = project(demo, row.project);
  if (row.kind === 'project') return p.lead;
  return p.workers.find((w) => w.id === row.worker) ?? null;
}

function withProject(demo: Demo, name: string, change: (p: Project) => Project): Demo {
  return { ...demo, projects: demo.projects.map((p) => (p.name === name ? change(p) : p)) };
}

function clampCursor(demo: Demo): Demo {
  return { ...demo, cursor: Math.min(demo.cursor, rows(demo).length - 1) };
}

function startLead(demo: Demo, name: string): Demo {
  return withProject(demo, name, (p) => ({
    ...p,
    lead: agent('lead', 'claude:opus:medium', 'waiting', 4_000, [
      { role: 'agent', text: `Lead started in ${name}. No workers are running and main is clean. What should I work on?` },
    ]),
  }));
}

function cycleProject(demo: Demo, step: 1 | -1): Demo {
  const running = demo.projects.filter((p) => p.lead);
  if (!running.length) return demo;
  const shown = demo.shown.kind === 'panel' ? -1 : running.findIndex((p) => p.name === (demo.shown as { project: string }).project);
  const next = running[(shown + step + running.length) % running.length];
  return { ...demo, shown: { kind: 'project', project: next.name } };
}

function onExplorerKey(demo: Demo, key: string): Demo {
  const list = rows(demo);
  const row = list[demo.cursor];
  if (demo.confirm) {
    const target = demo.confirm;
    if (key !== 'y') return { ...demo, confirm: null };
    return clampCursor(close({ ...demo, confirm: null }, target));
  }
  switch (key) {
    case 'ArrowDown': return { ...demo, cursor: Math.min(demo.cursor + 1, list.length - 1) };
    case 'ArrowUp': return { ...demo, cursor: Math.max(demo.cursor - 1, 0) };
    case 'ArrowRight':
      return row.kind === 'project' && project(demo, row.project).workers.length
        ? withProject(demo, row.project, (p) => ({ ...p, expanded: true }))
        : demo;
    case 'ArrowLeft':
      if (row.kind === 'worker') return { ...demo, cursor: list.findIndex((r) => r.kind === 'project' && r.project === row.project) };
      return row.kind === 'project' ? withProject(demo, row.project, (p) => ({ ...p, expanded: false })) : demo;
    case 'Enter':
      if (row.kind === 'project' && !project(demo, row.project).lead) return { ...startLead(demo, row.project), shown: row };
      return { ...demo, shown: row };
    case '?': return { ...demo, shown: { kind: 'panel', panel: 'help' } };
    case 'Escape': return { ...demo, shown: row };
    case '[': return cycleProject(demo, -1);
    case ']': return cycleProject(demo, 1);
    case 'c': return row.kind === 'panel' || !agentOf(demo, row) ? demo : { ...demo, confirm: row };
    case 'Tab': return { ...demo, focus: 'view' };
    default: return demo;
  }
}

function close(demo: Demo, target: Row): Demo {
  if (target.kind === 'panel') return demo;
  const next = withProject(demo, target.project, (p) =>
    target.kind === 'worker'
      ? { ...p, workers: p.workers.filter((w) => w.id !== target.worker) }
      : { ...p, lead: null, workers: [], expanded: false },
  );
  const shownGone = demo.shown.kind !== 'panel' && demo.shown.project === target.project &&
    (target.kind === 'project' || sameRow(demo.shown, target));
  return shownGone ? { ...next, shown: { kind: 'project', project: target.project } } : next;
}

/** The staged reply: a spawn request adds a worker on the next model family. */
function reply(demo: Demo, row: Row, text: string): Demo {
  if (row.kind !== 'project' || !SPAWN_PATTERN.test(text)) {
    const answer = row.kind === 'worker'
      ? 'Noted. I will finish this step first; the lead reads my report when I say done.'
      : 'I would give that to one worker and have a reviewer from another family check it. Say go to spawn them.';
    // A lead that answers waits for the operator (the PWA's push); a worker keeps working.
    return append(demo, row, [{ role: 'agent', text: answer }], row.kind === 'worker' ? 'working' : 'waiting');
  }
  const model = SPAWN_MODELS[demo.spawned % SPAWN_MODELS.length];
  const id = `w${413 + demo.spawned}`;
  const spawned = withProject({ ...demo, spawned: demo.spawned + 1 }, row.project, (p) => ({
    ...p,
    expanded: true,
    workers: [...p.workers, agent(id, model, 'working', 0, [{ role: 'tool', text: `Reading the brief from ${p.name}'s lead` }])],
  }));
  return append(spawned, row, [
    { role: 'tool', text: `niles spawn worker --id ${id}` },
    { role: 'agent', text: `Spawned ${id} on ${model}. I will read its report when it says done.` },
  ], 'working');
}

function append(demo: Demo, row: Row, entries: Entry[], state: AgentState): Demo {
  if (row.kind === 'panel') return demo;
  return withProject(demo, row.project, (p) => {
    const add = (a: Agent): Agent => ({ ...a, state, transcript: [...a.transcript, ...entries] });
    return row.kind === 'project' && p.lead
      ? { ...p, lead: add(p.lead) }
      : { ...p, workers: p.workers.map((w) => (row.kind === 'worker' && w.id === row.worker ? add(w) : w)) };
  });
}

/** The operator's message to an agent and its staged answer; the terminal and the phone both send here. */
export function sendTo(demo: Demo, row: Row, text: string): Demo {
  if (!text || !agentOf(demo, row)) return demo;
  return reply(append(demo, row, [{ role: 'you', text }], 'working'), row, text);
}

function onViewKey(demo: Demo, key: string): Demo {
  if (key === 'Escape' || key === 'Tab') return { ...demo, focus: 'explorer' };
  if (key === 'Backspace') return { ...demo, draft: demo.draft.slice(0, -1) };
  if (key === 'Enter') {
    const text = demo.draft.trim();
    return text && agentOf(demo, demo.shown) ? sendTo({ ...demo, draft: '' }, demo.shown, text) : demo;
  }
  return key.length === 1 && demo.draft.length < 120 ? { ...demo, draft: demo.draft + key } : demo;
}

/** One key, as KeyboardEvent.key names it. */
export function press(demo: Demo, key: string): Demo {
  return demo.focus === 'view' ? onViewKey(demo, key) : onExplorerKey(demo, key);
}
