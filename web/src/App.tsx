import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { API_VERSION, connectionError, type Session, sessions, sortedSessions } from './api';
import { Chat } from './Chat';
import { Notifications } from './push';
import { selectedSession, sessionHash } from './notification';

const SESSION_REFRESH_MS = 15_000;
export function App() {
  const [items, setItems] = useState<Session[]>([]);
  const [selected, setSelected] = useState<string>(selectedSession(window.location.hash));
  const [version, setVersion] = useState<number>();
  const [error, setError] = useState('');
  const [loaded, setLoaded] = useState(false);
  const statusRevision = useRef(0);
  useEffect(() => {
    const select = () => setSelected(selectedSession(window.location.hash));
    window.addEventListener('hashchange', select);
    return () => window.removeEventListener('hashchange', select);
  }, []);
  useEffect(() => {
    const controller = new AbortController();
    let inFlight = false;
    const refresh = async () => {
      if (inFlight) return;
      inFlight = true;
      const revision = statusRevision.current;
      try {
        const response = await sessions(controller.signal);
        if (controller.signal.aborted) return;
        setVersion(response.api_version);
        // An SSE status received after this request started is newer than its snapshot.
        if (revision === statusRevision.current) setItems(response.sessions);
        setError(''); setLoaded(true);
      } catch (cause) {
        if (!controller.signal.aborted) setError(connectionError(cause));
      } finally { inFlight = false; }
    };
    void refresh();
    const timer = setInterval(() => { void refresh(); }, SESSION_REFRESH_MS);
    window.addEventListener('focus', refresh);
    return () => { controller.abort(); clearInterval(timer); window.removeEventListener('focus', refresh); };
  }, []);
  const onStatus = useCallback((status: Session) => {
    statusRevision.current += 1;
    setItems(old => old.map(item => item.id === status.id ? status : item));
  }, []);
  const current = items.find(item => item.id === selected);
  const compatible = version === API_VERSION;
  return <div class="app">
    <aside><header><h1>Niles</h1><small>Your sessions</small></header>
      {compatible && <Notifications />}
      {error && <p class="banner" role="alert">{error}</p>}
      {version !== undefined && !compatible && <p class="banner" role="alert">Update the web app. API version {version} requires a matching release (this app supports {API_VERSION}).</p>}
      <nav aria-label="Sessions">{sortedSessions(items).map(item => <button key={item.id}
        class={`session ${selected === item.id ? 'selected' : ''}`} onClick={() => { window.location.hash = sessionHash(item.id); setSelected(item.id); }} aria-pressed={selected === item.id}>
        <strong>{item.project}</strong><span class="badge">Agent · {item.agent.family} · {item.agent.entry_skill}</span>
        <span>{item.waiting ? '⚠ Waiting · ' : ''}{item.worker_count} workers</span>
      </button>)}</nav>
      {loaded && compatible && items.length === 0 && <p class="empty">No live sessions.</p>}
      {!loaded && !error && <p class="empty">Connecting…</p>}
    </aside>
    {compatible && current ? <Chat key={current.id} session={current} onStatus={onStatus} /> :
      <main class="empty">{version !== undefined && !compatible ? 'Install the matching web release to continue.' : selected && !current ? 'This session has ended.' : 'Choose a session to chat with its agent.'}</main>}
  </div>;
}
