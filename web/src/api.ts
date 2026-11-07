import { z } from 'zod';

export const API_VERSION = 1;
export const sessionSchema = z.object({
  id: z.string().min(1), project: z.string(),
  agent: z.object({ family: z.string(), model: z.string().optional(), entry_skill: z.string() }),
  waiting: z.boolean(), worker_count: z.number().int().nonnegative(),
});
export type Session = z.infer<typeof sessionSchema>;
export const sessionsSchema = z.object({ api_version: z.number().int(), sessions: z.array(sessionSchema) });
export const projectSchema = z.object({
  name: z.string().min(1), lead_state: z.enum(['working', 'starting', 'waiting']).optional(), state: z.enum(['missing', 'not running', 'running', 'starting', 'waiting']),
  workers: z.array(z.object({ model: z.string().optional(), id: z.string().min(1), skill: z.string().regex(/^[A-Za-z0-9_-]+$/), lost: z.boolean(), state: z.enum(['idle', 'working', 'starting']).optional() })),
});
export type Project = z.infer<typeof projectSchema>;
const projectsSchema = z.object({ api_version: z.number().int(), projects: z.array(projectSchema) });
export async function projects(signal?: AbortSignal, prefix = '') {
  const result = await getJson(`${prefix}/api/projects`, projectsSchema, signal);
  return result.state === 'ready' ? result.data : { api_version: result.version, projects: [] };
}
export const turnSchema = z.object({
  id: z.string().min(1), role: z.enum(['agent', 'user']), text: z.string(),
});
export type Turn = z.infer<typeof turnSchema>;
export const RECENT_TURN_LIMIT = 200;
export const snapshotSchema = z.object({ turns: z.array(turnSchema).max(RECENT_TURN_LIMIT), truncated: z.boolean() });
export type Message = { text: string };
export type Connection = 'live' | 'reconnecting' | 'catching up' | 'disconnected';

export class ApiError extends Error {
  constructor(readonly status: number, readonly detail?: string) { super(detail ?? `API request failed (${status})`); }
}
export type Incompatible = { state: 'incompatible'; version: number };
const apiVersionSchema = z.object({ api_version: z.number().int() });
export function incompatibleMessage(version: number) {
  return `Update the web app. API version ${version} requires a matching release (this app supports ${API_VERSION}).`;
}
export async function getJson<T>(path: string, schema: z.ZodType<T>, signal?: AbortSignal): Promise<{ state: 'ready'; data: T } | Incompatible> {
  const response = await fetch(path, { cache: 'no-store', signal });
  if (!response.ok) throw new ApiError(response.status);
  const data: unknown = await response.json();
  const { api_version } = apiVersionSchema.parse(data);
  return api_version === API_VERSION ? { state: 'ready', data: schema.parse(data) }
    : { state: 'incompatible', version: api_version };
}
export async function sessions(signal?: AbortSignal, prefix = '') {
  const result = await getJson(`${prefix}/api/sessions`, sessionsSchema, signal);
  return result.state === 'ready' ? result.data : { api_version: result.version, sessions: [] };
}
export async function sendMessage(base: string, message: Message) {
  const response = await fetch(`${base}/message`, {
    method: 'POST', cache: 'no-store', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(message),
  });
  if (response.status !== 200) throw new ApiError(response.status);
}
export function upsertTurn(turns: Turn[], incoming: Turn): Turn[] {
  const index = turns.findIndex(turn => turn.id === incoming.id);
  return index < 0 ? [...turns, incoming] : turns.map((turn, i) => i === index ? incoming : turn);
}
export const TAILNET_UNREACHABLE = 'Cannot reach Niles; your tailnet may be disconnected. Reconnect Tailscale and try again.';
export function connectionError(cause: unknown): string {
  if (cause instanceof ApiError) return cause.message;
  return cause instanceof TypeError ? TAILNET_UNREACHABLE : `Invalid API response: ${String(cause)}`;
}

export const leadBase = (id: string, prefix = '') => `${prefix}/api/sessions/${encodeURIComponent(id)}`;
export const workerBase = (project: string, id: string, prefix = '') => `${prefix}/api/projects/${encodeURIComponent(project)}/workers/${encodeURIComponent(id)}`;

async function request(method: 'PUT' | 'DELETE', url: string) {
  const response = await fetch(url, { method, cache: 'no-store' });
  if (response.status === 200 || response.status === 404) return;
  throw new ApiError(response.status, response.status === 409 ? await response.text() : undefined);
}
export const startProject = (name: string, prefix = '') => request('PUT', `${prefix}/api/projects/${encodeURIComponent(name)}/session`);
export const stopProject = (name: string, prefix = '') => request('DELETE', `${prefix}/api/projects/${encodeURIComponent(name)}/session`);
export const closeWorker = (project: string, id: string, prefix = '') => request('DELETE', workerBase(project, id, prefix));
