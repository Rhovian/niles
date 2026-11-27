import { useState } from 'preact/hooks';
import { authorizationUrl, type OAuthSettings } from './auth';

export function SignIn({ name, prefix, settings }: { name: string; prefix: string; settings: OAuthSettings }) {
  const [signingIn, setSigningIn] = useState(false);
  const [error, setError] = useState('');
  async function start() {
    setSigningIn(true); setError('');
    try { location.assign(await authorizationUrl(prefix, settings)); }
    catch (cause) { setError(String(cause)); setSigningIn(false); }
  }
  return <p class="banner"><button disabled={signingIn} onClick={start}>Sign in to {name}</button>
    {error && <span role="alert">{error}</span>}</p>;
}
