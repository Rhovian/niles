import { useState } from 'preact/hooks';
import { ApiError } from './api';
import { type Host } from './hosts';

export function Wake({ host, refresh }: { host: Host; refresh: () => void }) {
  const [state, setState] = useState<'idle' | 'sending' | 'sent'>('idle');
  const [error, setError] = useState('');
  async function wake() {
    setState('sending'); setError('');
    try {
      const response = await fetch(`/api/hosts/${encodeURIComponent(host.id)}/wake`, { method: 'POST', cache: 'no-store', headers: { 'Content-Type': 'application/json' }, body: 'null' });
      if (response.status !== 204) throw new ApiError(response.status);
      setState('sent'); refresh();
    } catch (cause) { setError(String(cause)); setState('idle'); }
  }
  return <div><p>Asleep <button disabled={!host.id || state === 'sending'} onClick={() => void wake()}>Wake {host.name}</button></p>
    {state !== 'idle' && <p role="status">{state === 'sending' ? 'Sending wake request…' : 'Wake packet sent. Waiting for host…'}</p>}
    {error && <p role="alert">{error}</p>}
  </div>;
}
