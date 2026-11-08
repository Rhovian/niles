import { describe, expect, it } from 'vitest';
import { crateVersion, roadmap } from './docs';

describe('docs', () => {
  it('joins a roadmap bullet with its continuation and drops code ticks', () => {
    const md = '# Roadmap\n\nIntro.\n\n- **Pipelines:** set how `work` moves,\n  per project.\n- **TUI:** more.\n';
    expect(roadmap(md)).toEqual([
      { title: 'Pipelines', detail: 'set how work moves, per project.' },
      { title: 'TUI', detail: 'more.' },
    ]);
    expect(() => roadmap('# Roadmap\n')).toThrow(/bullets/);
  });

  it('takes the version from the package table only', () => {
    expect(crateVersion('[workspace]\n\n[package]\nname = "niles"\nversion = "1.13.0"\n')).toBe('1.13.0');
    expect(() => crateVersion('[dependencies]\nversion = "1"\n')).toThrow(/version/);
  });
});
