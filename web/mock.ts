import { z } from 'zod';
import type { Plugin } from 'vite';
import type { ServerResponse } from 'node:http';
import { API_VERSION, RECENT_TURN_LIMIT, type Session, type Turn } from './src/api';

export function mockApi(): Plugin {
  const messageSchema = z.object({ text: z.string().min(1) });
  const sessions: Session[] = [
    { id: 'demo', project: 'niles', agent: { family: 'codex', entry_skill: 'lead' }, waiting: true, worker_count: 2 },
    { id: 'solo', project: 'notes', agent: { family: 'claude', entry_skill: 'solo' }, waiting: false, worker_count: 0 },
  ];
  const transcripts = new Map(sessions.map(session => [session.id, [{
    id: `${session.id}-welcome`, role: 'agent' as const,
    text: '## Ready when you are\nThis is the **local mock API**. Send a message to try the chat.\n\n```sh\nniles workers\n```',
  }] as Turn[]]));
  const streams = new Map<string, Set<ServerResponse>>(sessions.map(session => [session.id, new Set()]));
  const emit = (response: ServerResponse, name: string, data: unknown) => response.write(`event: ${name}\ndata: ${JSON.stringify(data)}\n\n`);
  return { name: 'niles-mock-api', configureServer(server) {
    server.middlewares.use(async (request, response, next) => {
      const path = new URL(request.url!, 'http://localhost').pathname;
      if (!path.startsWith('/api/')) return next();
      response.setHeader('Cache-Control', 'no-store');
      const json = (status: number, body: unknown) => {
        response.writeHead(status, { 'Content-Type': 'application/json' }); response.end(JSON.stringify(body));
      };
      if (path === '/api/projects' && request.method === 'GET') return json(200, { api_version: API_VERSION, projects: [
        ...sessions.map(session => ({ name: session.project, state: session.waiting ? 'waiting' : 'running', workers: session.worker_count ? [{ id: 'nav', skill: 'worker', lost: false }, { id: 'review', skill: 'reviewer', lost: true }] : [] })),
        { name: 'offline', state: 'not running', workers: [] },
      ] });
      if (path === '/api/sessions' && request.method === 'GET') return json(200, { api_version: API_VERSION, sessions });
      // #318 owns VAPID/subscription behavior; mock refuses rather than pretending to subscribe.
      if (path.startsWith('/api/push/')) return json(501, { error: 'Push is not implemented in this mock (#318)' });
      if (/^\/api\/projects\/[^/]+\/(session|workers\/)/.test(path)) return json(501, { error: 'Project control and worker chat are not implemented in this mock' });
      const match = /^\/api\/sessions\/([^/]+)\/(events|message)$/.exec(path);
      const session = sessions.find(item => item.id === match?.[1]);
      if (!match || !session) return json(404, { error: 'Unknown endpoint or session' });
      const turns = transcripts.get(session.id)!;
      const clients = streams.get(session.id)!;
      if (match[2] === 'events' && request.method === 'GET') {
        response.writeHead(200, { 'Content-Type': 'text/event-stream', Connection: 'keep-alive' });
        clients.add(response); emit(response, 'snapshot', { turns: turns.slice(-RECENT_TURN_LIMIT), truncated: turns.length > RECENT_TURN_LIMIT }); emit(response, 'status', session);
        response.on('close', () => clients.delete(response));
        return;
      }
      if (match[2] === 'message' && request.method === 'POST') {
        let body = '';
        for await (const chunk of request) body += String(chunk);
        let input: unknown;
        try { input = JSON.parse(body); } catch { return json(400, { error: 'Invalid JSON' }); }
        const parsed = messageSchema.safeParse(input);
        if (!parsed.success) return json(400, { error: parsed.error.message });
        const message = parsed.data;
        const user: Turn = { id: crypto.randomUUID(), role: 'user', ...message };
        const agent: Turn = { id: crypto.randomUUID(), role: 'agent', text: `Received:\n\n${message.text}` };
        turns.push(user, agent); session.waiting = false;
        for (const client of clients) { emit(client, 'turn', user); emit(client, 'turn', agent); emit(client, 'status', session); }
        return json(200, {});
      }
      return json(405, { error: 'Method not allowed' });
    });
  } };
}
