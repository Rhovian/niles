import { render } from 'preact';
import { App } from './App';
import { finishSignIn } from './auth';
import { clearTelemetry } from './telemetry-data';
import './style.css';

void finishSignIn().then(resource => {
  if (resource) clearTelemetry(resource);
}).then(
  () => render(<App />, document.getElementById('app')!),
  cause => render(<App signInError={String(cause)} />, document.getElementById('app')!),
);
if (import.meta.env.PROD && 'serviceWorker' in navigator) {
  window.addEventListener('load', () => {
    void navigator.serviceWorker.register('/sw.js').catch(cause => {
      const warning = document.createElement('p');
      warning.className = 'banner';
      warning.setAttribute('role', 'alert');
      warning.textContent = `Offline shell could not be installed: ${String(cause)}`;
      document.body.prepend(warning);
    });
  });
}
