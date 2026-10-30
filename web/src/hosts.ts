import { z } from 'zod';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { API_VERSION, ApiError, connectionError, projects, sessions, type Project, type Session } from './api';

// Zod owns JSON validation; the refinement enforces our manifest's unique routing IDs.
// https://github.com/colinhacks/zod/tree/v4.6.7/packages/zod/src/v4
export const hostsSchema = z.object({ hosts: z.array(z.object({
  id: z.string().regex(/^[A-Za-z0-9][A-Za-z0-9_-]*$/), name: z.string().trim().min(1),
})).min(1).refine(hosts => new Set(hosts.map(host => host.id)).size === hosts.length, 'Duplicate host ID') });
export type Host = z.infer<typeof hostsSchema>['hosts'][number];
const HOST_TIMEOUT_MS = 8_000;
const HOST_REFRESH_MS = 15_000;
const LOCAL_HOST: Host = { id: '', name: 'This host' };
export const hostPrefix = (host: Host) => host.id ? `/h/${host.id}` : '';
export { qualified } from './notification';
type Availability = { state: 'loading' | 'asleep' } | { state: 'error'; error: string } |
  { state: 'incompatible'; version: number } | { state: 'ready'; projects: Project[]; sessions: Session[] };
export type HostState = Host & Availability;

export async function configuredHosts(signal: AbortSignal): Promise<Host[]> {
  const response = await fetch('/hosts.json', { cache: 'no-store', signal });
  if (response.status === 404) return [LOCAL_HOST];
  if (!response.ok) throw new ApiError(response.status);
  return hostsSchema.parse(await response.json()).hosts;
}
export function useHosts() {
  const [hosts, setHosts] = useState<HostState[]>([]);
  const [error, setError] = useState('');
  const refresh = useRef<() => void>(() => {});
  const revisions = useRef(new Map<string, number>());
  const onStatus = useCallback((hostId: string, status: Session) => {
    revisions.current.set(hostId, (revisions.current.get(hostId) ?? 0) + 1);
    setHosts(old => old.map(item => item.id === hostId && item.state === 'ready'
      ? { ...item, sessions: item.sessions.map(session => session.id === status.id ? status : session) } : item));
  }, []);
  useEffect(() => {
    const controller = new AbortController();
    const inFlight = new Set<string>();
    const update = (host: Host, availability: Availability) => {
      if (!controller.signal.aborted) setHosts(old => old.map(item => item.id === host.id ? { ...host, ...availability } : item));
    };
    const load = async (host: Host) => {
      if (inFlight.has(host.id)) return;
      inFlight.add(host.id);
      const revision = revisions.current.get(host.id);
      const request = new AbortController();
      const cancel = () => request.abort();
      controller.signal.addEventListener('abort', cancel, { once: true });
      const timeout = setTimeout(() => request.abort(new DOMException('Host request timed out', 'TimeoutError')), HOST_TIMEOUT_MS);
      try {
        const [response, navigation] = await Promise.all([
          sessions(request.signal, hostPrefix(host)), projects(request.signal, hostPrefix(host)),
        ]);
        const version = response.api_version === API_VERSION ? navigation.api_version : response.api_version;
        if (version !== API_VERSION) update(host, { state: 'incompatible', version });
        else if (!controller.signal.aborted) setHosts(old => old.map(item => item.id === host.id ? {
          ...host, state: 'ready', projects: navigation.projects,
          // An SSE status received after this request started is newer than its snapshot.
          sessions: revision !== revisions.current.get(host.id) && item.state === 'ready' ? item.sessions : response.sessions,
        } : item));
      } catch (cause) {
        update(host, cause instanceof TypeError || cause instanceof DOMException && cause.name === 'TimeoutError' || cause instanceof ApiError && [502, 503, 504].includes(cause.status)
          ? { state: 'asleep' } : { state: 'error', error: connectionError(cause) });
      } finally {
        clearTimeout(timeout); controller.signal.removeEventListener('abort', cancel);
        request.abort(); inFlight.delete(host.id);
      }
    };
    void configuredHosts(controller.signal).then(configured => {
      if (controller.signal.aborted) return;
      setHosts(configured.map(host => ({ ...host, state: 'loading' })));
      refresh.current = () => { for (const host of configured) void load(host); };
      refresh.current();
    }).catch(cause => { if (!controller.signal.aborted) setError(connectionError(cause)); });
    const focus = () => refresh.current();
    const timer = setInterval(focus, HOST_REFRESH_MS);
    window.addEventListener('focus', focus);
    return () => { controller.abort(); clearInterval(timer); window.removeEventListener('focus', focus); };
  }, []);
  return { hosts, error, refresh: () => refresh.current(), onStatus };
}
