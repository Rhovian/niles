// @vitest-environment node
import { expect, it } from 'vitest';
import { createServer } from 'vite';
import { fileURLToPath } from 'node:url';

it('serves the mock JSON/SSE contract and keeps script execution external', async () => {
  const server = await createServer({ root: fileURLToPath(new URL('.', import.meta.url)), mode: 'mock', server: { port: 0 } });
  await server.listen();
  const base = server.resolvedUrls!.local[0];
  const controller = new AbortController();
  try {
    const html = await (await fetch(base)).text();
    expect(html).toContain("script-src 'self'");
    expect(html).not.toMatch(/<script(?![^>]*src=)[^>]*>\s*\S/);
    const list = await fetch(`${base}api/sessions`);
    expect(list.headers.get('cache-control')).toBe('no-store');
    expect(await list.json()).toMatchObject({ api_version: 1, sessions: [{ agent: { entry_skill: 'lead' } }, { agent: { entry_skill: 'solo' } }] });
    const projects = await fetch(`${base}api/projects`);
    expect(projects.headers.get('cache-control')).toBe('no-store');
    expect(await projects.json()).toMatchObject({ api_version: 1, projects: [{ name: 'niles', state: 'waiting', workers: [{ role: 'worker', lost: false }, { role: 'reviewer', lost: true }] }, { name: 'notes' }, { name: 'offline', state: 'not running' }] });
    const events = await fetch(`${base}api/sessions/solo/events`, { signal: controller.signal });
    expect(events.headers.get('content-type')).toBe('text/event-stream');
    const reader = events.body!.getReader();
    expect(new TextDecoder().decode((await reader.read()).value)).toContain('event: snapshot');
    const message = { text: 'hello' };
    const post = () => fetch(`${base}api/sessions/solo/message`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(message) });
    expect((await post()).status).toBe(200);
    expect(new TextDecoder().decode((await reader.read()).value)).toContain(message.text);
    expect((await fetch(`${base}api/push/vapid`)).status).toBe(501);
    controller.abort();
  } finally {
    controller.abort();
    await server.close();
  }
});
