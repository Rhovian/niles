import { render } from 'preact';
import { App } from './App';
import './style.css';

render(<App />, document.getElementById('app')!);
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
