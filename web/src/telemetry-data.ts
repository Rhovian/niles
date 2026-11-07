import { z } from 'zod';
import { useEffect, useState } from 'preact/hooks';
import { API_VERSION, ApiError, connectionError } from './api';
import { configuredHosts, hostPrefix, hostFailure, HOST_TIMEOUT_MS, type Host } from './hosts';

const timestamp = z.iso.datetime({ offset: true });
const tokens = z.number().int().nonnegative();
export const telemetrySchema = z.object({
  api_version: z.literal(API_VERSION), collected_at: timestamp,
  sessions: z.array(z.object({
    project: z.string(), id: z.string(), entry: z.boolean(), family: z.string(), model: z.string().nullable(),
    created_at: timestamp, closed_at: timestamp.nullable(),
    buckets: z.record(timestamp.refine(at => Date.parse(at) % 900_000 === 0, 'Unaligned bucket'), tokens),
  })),
});
export type Snapshot = z.infer<typeof telemetrySchema>;
type Availability = { state: 'loading' | 'ready' | 'asleep' } | { state: 'error'; error: string };
export type Machine = Host & Availability & { snapshot?: Snapshot; cacheWarning?: string };
const cacheKey = (host: Host) => `niles.telemetry.v${API_VERSION}:${host.id}`;

// Cached formats are disposable: schema/API upgrades and malformed cache entries invalidate
// the cache, not the host. Zod validates but does not migrate or persist browser storage.
// https://github.com/colinhacks/zod/tree/v4.6.7/packages/zod/src/v4
function readCache(host: Host): Pick<Machine, 'snapshot' | 'cacheWarning'> {
  const cached = localStorage.getItem(cacheKey(host));
  if (cached === null) return {};
  try {
    const parsed = telemetrySchema.safeParse(JSON.parse(cached));
    if (parsed.success) return { snapshot: parsed.data };
  } catch (cause) {
    if (!(cause instanceof SyntaxError)) throw cause;
  }
  localStorage.removeItem(cacheKey(host));
  return { cacheWarning: 'An outdated or invalid cached snapshot was discarded.' };
}

export function useTelemetry() {
  const [machines, setMachines] = useState<Machine[]>([]);
  const [error, setError] = useState('');
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    const update = (host: Host, availability: Availability, details?: Pick<Machine, 'snapshot' | 'cacheWarning'>) => {
      if (!controller.signal.aborted) setMachines(old => old.map(machine => machine.id === host.id
        ? { ...machine, ...availability, ...details } : machine));
    };
    const load = async (host: Host) => {
      try {
        update(host, { state: 'loading' }, readCache(host));
        const signal = AbortSignal.any([controller.signal, AbortSignal.timeout(HOST_TIMEOUT_MS)]);
        const response = await fetch(`${hostPrefix(host)}/api/telemetry`, { cache: 'no-store', signal });
        if (!response.ok) throw new ApiError(response.status);
        const snapshot = telemetrySchema.parse(await response.json());
        if (controller.signal.aborted) return;
        try {
          localStorage.setItem(cacheKey(host), JSON.stringify(snapshot));
        } catch (cause) {
          update(host, { state: 'error', error: `Cannot cache telemetry: ${String(cause)}` }, { snapshot });
          return;
        }
        update(host, { state: 'ready' }, { snapshot });
      } catch (cause) {
        update(host, hostFailure(cause));
      }
    };
    void configuredHosts(controller.signal).then(async hosts => {
      if (controller.signal.aborted) return;
      setError('');
      setMachines(hosts.map(host => ({ ...host, state: 'loading' })));
      await Promise.all(hosts.map(load));
    }).catch(cause => { if (!controller.signal.aborted) setError(connectionError(cause)); });
    return () => controller.abort();
  }, [revision]);
  return { machines, error, refresh: () => setRevision(old => old + 1) };
}
