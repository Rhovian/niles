import { describe, expect, it } from 'vitest';
import { initialDemo, press, project, rows, type Demo } from './model';
import { hint, view, wrap } from './view';

const keys = (demo: Demo, ...seq: string[]) => seq.reduce(press, demo);
const text = (lines: { t: string }[][]) => lines.map((l) => l.map((s) => s.t).join('')).join('\n');
const type = (demo: Demo, words: string) => keys(demo, ...words.split(''));

describe('explorer keys', () => {
  it('moves the cursor within the rows', () => {
    const demo = initialDemo();
    expect(keys(demo, 'ArrowUp').cursor).toBe(0);
    expect(keys(demo, ...Array(50).fill('ArrowDown')).cursor).toBe(rows(demo).length - 1);
  });

  it('collapses and expands a project with workers', () => {
    const collapsed = keys(initialDemo(), 'ArrowLeft');
    expect(project(collapsed, 'billing').expanded).toBe(false);
    expect(rows(collapsed).some((r) => r.kind === 'worker')).toBe(false);
    expect(project(keys(collapsed, 'ArrowRight'), 'billing').expanded).toBe(true);
  });

  it('starts a stopped project on Enter and shows its lead', () => {
    const at = rows(initialDemo()).findIndex((r) => r.kind === 'project' && r.project === 'mobile');
    const demo = keys({ ...initialDemo(), cursor: at }, 'Enter');
    expect(project(demo, 'mobile').lead?.state).toBe('waiting');
    expect(demo.shown).toEqual({ kind: 'project', project: 'mobile' });
    expect(text(view(demo, 60))).toContain('Lead started in mobile');
  });

  it('shows help on ? and the selection again on Escape', () => {
    const help = keys(initialDemo(), '?');
    expect(help.shown).toEqual({ kind: 'panel', panel: 'help' });
    expect(keys(help, 'Escape').shown).toEqual({ kind: 'project', project: 'billing' });
  });

  it('cycles running projects with ] and [', () => {
    expect(keys(initialDemo(), ']').shown).toEqual({ kind: 'project', project: 'docs' });
    expect(keys(initialDemo(), '[', '[').shown).toEqual({ kind: 'project', project: 'billing' });
  });

  it('closes a worker only after y', () => {
    const asked = keys(initialDemo(), 'ArrowDown', 'c');
    expect(text([hint(asked)])).toContain('close w412?');
    expect(project(keys(asked, 'n'), 'billing').workers).toHaveLength(1);
    const closed = keys(asked, 'y');
    expect(project(closed, 'billing').workers).toHaveLength(0);
    expect(closed.cursor).toBeLessThan(rows(closed).length);
  });
});

describe('typing to an agent', () => {
  it('keeps keys in the draft until Enter, and Escape returns to the explorer', () => {
    const typing = type(keys(initialDemo(), 'Tab'), 'hi');
    expect(typing.draft).toBe('hi');
    expect(keys(typing, 'Backspace').draft).toBe('h');
    expect(keys(typing, 'Escape').focus).toBe('explorer');
  });

  it('spawns a worker on the next family when asked for a review', () => {
    const sent = keys(type(keys(initialDemo(), 'Tab'), 'please review it'), 'Enter');
    const billing = project(sent, 'billing');
    expect(billing.workers.map((w) => w.id)).toEqual(['w412', 'w413']);
    expect(billing.workers[1].model).toBe('codex:gpt-6.1-sol:medium');
    expect(sent.draft).toBe('');
    const again = keys(type(sent, 'go'), 'Enter');
    expect(project(again, 'billing').workers[2].model).toBe('claude:opus:high');
  });

  it('answers without spawning otherwise, and ignores an empty send', () => {
    const start = keys(initialDemo(), 'Tab');
    expect(keys(start, 'Enter')).toEqual(start);
    const sent = keys(type(start, 'status?'), 'Enter');
    expect(project(sent, 'billing').workers).toHaveLength(1);
    expect(text(view(sent, 80))).toContain('❯ status?');
  });
});

describe('wrap', () => {
  it('breaks on spaces within the width', () => {
    expect(wrap('aaa bbb ccc', 7)).toEqual(['aaa bbb', 'ccc']);
  });
});
