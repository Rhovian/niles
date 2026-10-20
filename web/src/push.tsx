import { useState } from 'preact/hooks';
import { z } from 'zod';
import { ApiError } from './api';

export function Notifications() {
  const [state, setState] = useState<'idle' | 'busy' | 'enabled'>('idle');
  const [error, setError] = useState('');
  async function enable() {
    setState('busy'); setError('');
    try {
      if (!('serviceWorker' in navigator && 'PushManager' in window && 'Notification' in window)) throw new Error('Web Push is unavailable. On iOS 16.4+, install Niles on your home screen first.');
      if (await Notification.requestPermission() !== 'granted') throw new Error('Notification permission was not granted.');
      const registration = await navigator.serviceWorker.getRegistration();
      if (!registration?.active) throw new Error('Open the installed production app and try again once its offline shell is ready.');
      const response = await fetch('/api/push/vapid', { cache: 'no-store' });
      if (!response.ok) throw new ApiError(response.status);
      const { public_key } = z.object({ public_key: z.string() }).parse(await response.json());
      const subscription = await registration.pushManager.getSubscription() ?? await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: public_key });
      const saved = await fetch('/api/push/subscribe', { method: 'POST', cache: 'no-store', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(subscription.toJSON()) });
      if (saved.status !== 200) throw new ApiError(saved.status);
      setState('enabled');
    } catch (cause) { setError(String(cause)); setState('idle'); }
  }
  return <div><button disabled={state !== 'idle'} onClick={enable}>{state === 'enabled' ? 'Notifications enabled' : state === 'busy' ? 'Enabling…' : 'Enable notifications'}</button>
    {error && <p role="alert" class="banner">{error}</p>}</div>;
}
