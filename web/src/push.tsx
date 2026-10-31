import { useState } from 'preact/hooks';
import { z } from 'zod';
import { ApiError } from './api';
import { hostPrefix, type Host } from './hosts';

export function Notifications({ hosts }: { hosts: Host[] }) {
  const [state, setState] = useState<'idle' | 'busy' | 'enabled'>('idle');
  const [error, setError] = useState('');
  async function enable() {
    setState('busy'); setError('');
    try {
      if (!('serviceWorker' in navigator && 'PushManager' in window && 'Notification' in window)) throw new Error('Web Push is unavailable. On iOS 16.4+, install Niles on your home screen first.');
      if (await Notification.requestPermission() !== 'granted') throw new Error('Notification permission was not granted.');
      const registration = await navigator.serviceWorker.getRegistration();
      if (!registration?.active) throw new Error('Open the installed production app and try again once its offline shell is ready.');
      const keys = await Promise.all(hosts.map(async host => {
        const response = await fetch(`${hostPrefix(host)}/api/push/vapid`, { cache: 'no-store', signal: AbortSignal.timeout(8_000) });
        if (!response.ok) throw new Error(`${host.name}: ${new ApiError(response.status).message}`);
        return z.object({ public_key: z.string().min(1) }).parse(await response.json()).public_key;
      }));
      const publicKey = keys[0];
      if (!publicKey || keys.some(key => key !== publicKey)) throw new Error('All hosts must use the same VAPID key. Copy vapid.json to every host and restart Niles.');
      const existing = await registration.pushManager.getSubscription();
      if (existing?.options.applicationServerKey) {
        const key = new Uint8Array(existing.options.applicationServerKey);
        // PushSubscriptionOptions exposes bytes; atob only accepts standard Base64, not base64url.
        // TypeScript 5.9.3: src/lib/dom.generated.d.ts (PushSubscriptionOptions, WindowOrWorkerGlobalScope).
        const expected = Uint8Array.from(atob(publicKey.replace(/-/g, '+').replace(/_/g, '/')), char => char.charCodeAt(0));
        if (key.length !== expected.length || key.some((byte, index) => byte !== expected[index])) throw new Error('This browser uses a different VAPID key. Restore the shared key or remove the old browser subscription first.');
      }
      const subscription = existing ?? await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: publicKey });
      const results = await Promise.allSettled(hosts.map(async host => {
        const saved = await fetch(`${hostPrefix(host)}/api/push/subscribe`, { method: 'POST', cache: 'no-store', signal: AbortSignal.timeout(8_000), headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...subscription.toJSON(), host: host.id ? { id: host.id, name: host.name } : null }) });
        if (saved.status !== 200) throw new ApiError(saved.status);
      }));
      const failures = results.flatMap((result, index) => result.status === 'rejected' ? [`${hosts[index].name}: ${String(result.reason)}`] : []);
      if (failures.length) throw new Error(failures.join('; '));
      setState('enabled');
    } catch (cause) { setError(String(cause)); setState('idle'); }
  }
  return <div><button disabled={state !== 'idle'} onClick={enable}>{state === 'enabled' ? 'Notifications enabled' : state === 'busy' ? 'Enabling…' : 'Enable notifications'}</button>
    {error && <p role="alert" class="banner">{error}</p>}</div>;
}
