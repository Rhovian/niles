import { z } from 'zod';

export const pushPayload = z.object({ session_id: z.string().min(1), project: z.string(), reason: z.string() });
export function sessionHash(id: string) { return `#/sessions/${encodeURIComponent(id)}`; }
export function selectedSession(hash: string): string | undefined {
  const match = /^#\/sessions\/([^/]+)$/.exec(hash);
  if (!match) return undefined;
  try { return decodeURIComponent(match[1]); } catch { return undefined; }
}
