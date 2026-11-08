import { describe, expect, it } from 'vitest';
import { initialDemo, project, sendTo } from './model';
import { act, initialPhone, notifications, phoneView, PHONE_COLS } from './phone';
import { cols, type Line } from './view';

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
    // The docs lead runs on codex, so even the page's first spawn reviews from another family.
    expect(project(demo, 'docs').workers[0].model).toBe('claude:opus:high');
    const chat = text(phoneView(demo, act(initialPhone(), 'open:docs'), CLOCK).top);
    expect(chat).toContain('You · Sent');
    expect(chat).toContain('review it first');
    expect(chat).not.toContain('niles spawn');
  });

  it('keeps every frame line the frame width, emoji included', () => {
    // Emoji take two columns in two UTF-16 units, ☕ two in one, 𝐀 one in two: width must count columns.
    const demo = ['a'.repeat(80), '😀'.repeat(24), '☕'.repeat(20), '𝐀'.repeat(40)].reduce((d, text) => sendTo(d, docs, text), initialDemo());
    for (const phone of [initialPhone(), act(initialPhone(), 'menu'), act(initialPhone(), 'open:docs')]) {
      const { top, bottom } = phoneView(demo, phone, CLOCK);
      for (const line of [...top, ...bottom]) expect(cols(line.map((s) => s.t).join(''))).toBe(PHONE_COLS + 2);
    }
  });
});
