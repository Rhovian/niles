import { z } from 'zod';

export const API_VERSION = 1;
export const sessionSchema = z.object({
  id: z.string().min(1), project: z.string(),
  agent: z.object({ family: z.string(), entry_skill: z.string() }),
  waiting: z.boolean(), worker_count: z.number().int().nonnegative(),
});
export type Session = z.infer<typeof sessionSchema>;
export const sessionsSchema = z.object({ api_version: z.number().int(), sessions: z.array(sessionSchema) });
export const turnSchema = z.object({
  id: z.string().min(1), role: z.enum(['agent', 'user']), text: z.string(),
});
export type Turn = z.infer<typeof turnSchema>;
export const RECENT_TURN_LIMIT = 200;
export const snapshotSchema = z.object({ turns: z.array(turnSchema).max(RECENT_TURN_LIMIT), truncated: z.boolean() });
export type Message = { text: string };
export type Connection = 'live' | 'reconnecting' | 'catching up' | 'disconnected';

export class ApiError extends Error {
  constructor(readonly status: number) { super(`API request failed (${status})`); }
}
export async function sessions(signal?: AbortSignal) {
  const response = await fetch('/api/sessions', { cache: 'no-store', signal });
  if (!response.ok) throw new ApiError(response.status);
  const data: unknown = await response.json();
  const { api_version } = z.object({ api_version: z.number().int() }).parse(data);
  if (api_version !== API_VERSION) return { api_version, sessions: [] };
  return sessionsSchema.parse(data);
}
export async function sendMessage(id: string, message: Message) {
  const response = await fetch(`/api/sessions/${encodeURIComponent(id)}/message`, {
    method: 'POST', cache: 'no-store', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(message),
  });
  if (response.status !== 200) throw new ApiError(response.status);
}
export function upsertTurn(turns: Turn[], incoming: Turn): Turn[] {
  const index = turns.findIndex(turn => turn.id === incoming.id);
  return index < 0 ? [...turns, incoming] : turns.map((turn, i) => i === index ? incoming : turn);
}
export function sortedSessions(items: Session[]) {
  return [...items].sort((a, b) => Number(b.waiting) - Number(a.waiting) || a.project.localeCompare(b.project));
}

export function connectionError(cause: unknown): string {
  if (cause instanceof ApiError) return cause.message;
  return cause instanceof TypeError ? `Cannot reach Niles. Check your tailnet connection. ${String(cause)}` : `Invalid API response: ${String(cause)}`;
}
