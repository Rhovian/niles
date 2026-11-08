import { describe, expect, it } from 'vitest';
import { initialDemo, project, sendTo } from './model';
import { act, initialPhone, notifications, phoneView, PHONE_COLS } from './phone';
import type { Line } from './view';

const CLOCK = { time: '09:41', date: 'Friday 7 November' };
const text = (lines: Line[]) => lines.map((l) => l.map((s) => s.t).join('')).join('\n');
const docs = { kind: 'project', project: 'docs' } as const;

describe('phone', () => {
  it('pushes the waiting lead to the lock screen and opens its chat', () => {
    const demo = initialDemo();
    expect(notifications(demo)).toEqual(['docs']);
    const lock = phoneView(demo, initialPhone(), CLOCK);
    expect(text(lock.top)).toContain('docs: Agent waiting for');
    expect(lock.top.flat().find((s) => s.a)?.a).toBe('open:docs');
    expect(act(initialPhone(), 'open:docs')).toEqual({ screen: 'chat', open: docs });
    expect(() => act(initialPhone(), 'reboot')).toThrow(/unknown/);
  });

  it('sends into the shared session, so the terminal sees the worker it spawns', () => {
    const demo = sendTo(initialDemo(), docs, 'review it first');
    expect(project(demo, 'docs').workers.map((w) => w.id)).toEqual(['w413']);
    const chat = text(phoneView(demo, act(initialPhone(), 'open:docs'), CLOCK).top);
    expect(chat).toContain('You · Sent');
    expect(chat).toContain('review it first');
    expect(chat).not.toContain('niles spawn');
  });

  it('keeps every frame line the frame width', () => {
    const demo = sendTo(initialDemo(), docs, 'a'.repeat(80));
    for (const phone of [initialPhone(), act(initialPhone(), 'menu'), act(initialPhone(), 'open:docs')]) {
      const { top, bottom } = phoneView(demo, phone, CLOCK);
      for (const line of [...top, ...bottom]) expect([...line.map((s) => s.t).join('')]).toHaveLength(PHONE_COLS + 2);
    }
  });
});
