import { useState } from 'preact/hooks';
import { useTelemetry, type Snapshot } from './telemetry-data';

const DAYS = 7;
const BUCKET_MS = 900_000;
const DAY_MS = 86_400_000;
function spent(session: Snapshot['sessions'][number], since: number) {
  return Object.entries(session.buckets).reduce((sum, [at, tokens]) => Date.parse(at) >= since ? sum + tokens : sum, 0);
}
export function Telemetry() {
  const { machines, error, refresh } = useTelemetry();
  const [days, setDays] = useState(DAYS);
  const since = Math.floor((Date.now() - days * DAY_MS) / BUCKET_MS) * BUCKET_MS;
  const total = machines.reduce((sum, machine) => sum + (machine.snapshot?.sessions.reduce((sum, session) => sum + spent(session, since), 0) ?? 0), 0);
  return <main class="telemetry" aria-label="Telemetry">
    <header><div><h2>Usage across machines</h2><p>{total.toLocaleString()} tokens in cached snapshots</p></div><button onClick={refresh}>Refresh telemetry</button></header>
    <label>Time range<select value={days} onInput={event => setDays(Number(event.currentTarget.value))}>
      {[1, 7, 30].map(days => <option key={days} value={days}>Last {days} {days === 1 ? 'day' : 'days'}</option>)}
    </select></label>
    {error && <p role="alert">{error}</p>}
    {machines.map(machine => {
      const sessions = machine.snapshot?.sessions.filter(session => spent(session, since) > 0 || Date.parse(session.created_at) >= since);
      return <section key={machine.id} aria-label={`${machine.name} telemetry`}>
        <h3>{machine.name}</h3>
        {machine.state === 'error' ? <p role="alert">Failed: {machine.error}</p> :
          machine.state === 'asleep' ? <p>Asleep or unreachable</p> : machine.state === 'loading' ? <p>Connecting…</p> : <p>Connected</p>}
        {machine.cacheWarning && <p role="status">{machine.cacheWarning}</p>}
        {machine.snapshot ? <>
          <p>As of <time dateTime={machine.snapshot.collected_at}>{new Date(machine.snapshot.collected_at).toLocaleString()}</time></p>
          <p>{machine.snapshot.sessions.reduce((sum, session) => sum + spent(session, since), 0).toLocaleString()} tokens</p>
          <table><thead><tr><th>Project / session</th><th>Model</th><th>Tokens</th></tr></thead><tbody>
            {sessions?.map(session => <tr key={`${session.project}:${session.entry}:${session.id}:${session.created_at}`}>
              <td>{session.project} / {session.id}<small class="badge">{session.entry ? 'Lead' : 'Worker'} · Started {new Date(session.created_at).toLocaleString()}{session.closed_at && ` · Closed ${new Date(session.closed_at).toLocaleString()}`}</small></td>
              <td>{session.family} / {session.model ?? 'Default'}</td><td>{spent(session, since).toLocaleString()}</td>
            </tr>)}
          </tbody></table>
          {sessions?.length === 0 && <p>No usage in this range.</p>}
        </> : machine.state !== 'loading' && <p>No cached usage.</p>}
      </section>;
    })}
  </main>;
}
