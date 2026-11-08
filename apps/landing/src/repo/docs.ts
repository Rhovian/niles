// Facts the page reads from the repository at build time, so they cannot drift from it.

export type RoadmapItem = { title: string; detail: string };

/** ROADMAP.md's bullets: `- **Title:** detail`, the detail continuing on indented lines. */
export function roadmap(md: string): RoadmapItem[] {
  const items: RoadmapItem[] = [];
  for (const line of md.split('\n')) {
    const bullet = line.match(/^- \*\*(.+?):\*\* (.*)$/);
    if (bullet) items.push({ title: bullet[1], detail: bullet[2] });
    else if (/^\s+\S/.test(line) && items.length) items[items.length - 1].detail += ` ${line.trim()}`;
  }
  if (!items.length) throw new Error('ROADMAP.md has no "- **Title:** detail" bullets');
  return items.map((i) => ({ ...i, detail: i.detail.replaceAll('`', '') }));
}

/** The crate's version from Cargo.toml's [package] table. */
export function crateVersion(toml: string): string {
  const version = toml.match(/^\[package\][\s\S]*?^version = "([^"]+)"/m)?.[1];
  if (!version) throw new Error('Cargo.toml has no [package] version');
  return version;
}
