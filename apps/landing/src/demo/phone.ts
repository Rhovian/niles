// The remote PWA on a phone, drawn in box characters: a second view on the same staged session.
// Words follow apps/pwa/: notifications "Niles · <project>" / "<project>: Agent waiting for your input",
// bubbles "Agent" and "You · Sent", the composer "Message agent".

import { agentOf, type Demo, type Row } from './model';
import { cols, wrap, type Line, type Seg } from './view';

export type Phone = { screen: 'lock' | 'chat' | 'menu'; open: Exclude<Row, { kind: 'panel' }> | null };
export type Clock = { time: string; date: string };

/** Columns between the frame's borders, and screen rows between the toolbar and the composer. */
export const PHONE_COLS = 34;
export const PHONE_ROWS = 18;
const TEXT = PHONE_COLS - 2;
const BUBBLE = 24;

export const initialPhone = (): Phone => ({ screen: 'lock', open: null });

/** Projects whose lead waits for the operator: each is a push on the lock screen. */
export function notifications(demo: Demo): string[] {
  return demo.projects.filter((p) => p.lead?.state === 'waiting').map((p) => p.name);
}

/** `menu` toggles the project list, `lock` locks, `open:<project>` or `open:<project>/<worker>` opens a chat. */
export function act(phone: Phone, action: string): Phone {
  if (action === 'menu') return { ...phone, screen: phone.screen === 'menu' && phone.open ? 'chat' : 'menu' };
  if (action === 'lock') return { ...phone, screen: 'lock' };
  if (!action.startsWith('open:')) throw new Error(`unknown phone action ${action}`);
  const [project, worker] = action.slice('open:'.length).split('/');
  return { screen: 'chat', open: worker ? { kind: 'worker', project, worker } : { kind: 'project', project } };
}

const width = (segs: Seg[]) => segs.reduce((n, s) => n + cols(s.t), 0);
const row = (segs: Seg[], align: 'left' | 'right' | 'centre' = 'left'): Line => {
  const room = Math.max(0, TEXT - width(segs));
  const left = align === 'right' ? room : align === 'centre' ? Math.floor(room / 2) : 0;
  return [{ t: '│ ' + ' '.repeat(left), c: 'dim' }, ...segs, { t: ' '.repeat(room - left) + ' │', c: 'dim' }];
};
const rule = (l: string, r: string): Line => [{ t: l + '─'.repeat(PHONE_COLS) + r, c: 'dim' }];

function box(lines: Seg[][], inner: number): Seg[][] {
  return [
    [{ t: `┌${'─'.repeat(inner + 2)}┐`, c: 'dim' }],
    ...lines.map((segs) => [{ t: '│ ', c: 'dim' }, ...segs, { t: ' '.repeat(inner - width(segs)) + ' │', c: 'dim' }]),
    [{ t: `└${'─'.repeat(inner + 2)}┘`, c: 'dim' }],
  ];
}

function lockScreen(demo: Demo, clock: Clock): Seg[][] {
  const pushes = notifications(demo).flatMap((p) =>
    box([[{ t: `Niles · ${p}`, c: 'b', a: `open:${p}` }], ...wrap(`${p}: Agent waiting for your input`, TEXT - 4).map((t) => [{ t }])], TEXT - 4));
  return [[], [{ t: clock.time, c: 'h' }], [{ t: clock.date, c: 'dim' }], [], ...(pushes.length ? pushes : [[{ t: 'No notifications', c: 'dim' }]])];
}

function menuScreen(demo: Demo): Seg[][] {
  return demo.projects.flatMap((p) => p.lead
    ? [[{ t: p.lead.state === 'waiting' ? '⚠ ' : '● ', c: p.lead.state }, { t: p.name, c: 'b', a: `open:${p.name}` }],
      ...p.workers.map((w) => [{ t: '  └ worker ', c: 'dim' }, { t: w.id, a: `open:${p.name}/${w.id}` }])]
    : [[{ t: `  ${p.name}`, c: 'dim' }, { t: '  not running', c: 'dim' }]]);
}

function chatScreen(demo: Demo, open: Phone['open']): Seg[][] {
  const agent = open && agentOf(demo, open);
  if (!agent) return [[{ t: 'This session has ended.', c: 'dim' }]];
  // The remote API drops tool calls; only the operator's and the agent's words reach the phone.
  const lines = agent.transcript.filter((e) => e.role !== 'tool').flatMap((e) => {
    const text = wrap(e.text, BUBBLE);
    const bubble = [[{ t: e.role === 'you' ? 'You · Sent' : 'Agent', c: 'dim' }], ...box(text.map((t) => [{ t }]), Math.max(...text.map(cols)))];
    return e.role === 'you' ? bubble.map((segs) => [{ t: ' '.repeat(TEXT - width(segs)) }, ...segs]) : bubble;
  });
  return lines.slice(-PHONE_ROWS);
}

function toolbar(demo: Demo, phone: Phone): Line[] {
  if (phone.screen === 'lock') return [row([]), row([])];
  const menu: Seg = { t: '≡', c: 'brand', a: 'menu' };
  if (phone.screen === 'menu' || !phone.open) return [row([menu, { t: '  Projects', c: 'b' }]), row([])];
  const name = phone.open.kind === 'worker' ? `${phone.open.project} · ${phone.open.worker}` : `${phone.open.project} · lead`;
  return [row([menu, { t: `  ${name}`, c: 'b' }]), row([{ t: `${agentOf(demo, phone.open)?.model ?? 'ended'} · `, c: 'dim' }, { t: 'live', c: 'ok' }])];
}

/** The phone above its composer, and the frame below it; the composer itself is a real form. */
export function phoneView(demo: Demo, phone: Phone, clock: Clock): { top: Line[]; bottom: Line[] } {
  const body = phone.screen === 'lock' ? lockScreen(demo, clock) : phone.screen === 'menu' ? menuScreen(demo) : chatScreen(demo, phone.open);
  const align = phone.screen === 'lock' ? 'centre' : 'left';
  const chat = phone.screen === 'chat';
  const fill: Line[] = Array(Math.max(0, PHONE_ROWS - body.length)).fill(row([]));
  // A chat pins its latest turn to the composer, as the PWA follows the newest content.
  const screen = chat ? [...fill, ...body.map((segs) => row(segs))] : [...body.map((segs) => row(segs, align)), ...fill];
  return {
    top: [rule('╭', '╮'), row([{ t: clock.time }, { t: ' '.repeat(TEXT - clock.time.length - 4) }, { t: '▮▮▮▯', c: 'dim' }]), ...toolbar(demo, phone), rule('├', '┤'), ...screen, rule('├', '┤')],
    bottom: [...(chat ? [] : [row([{ t: phone.screen === 'lock' ? 'tap a notification' : 'pick a project or worker', c: 'dim' }], 'centre'), row([])]), rule('╰', '╯')],
  };
}
