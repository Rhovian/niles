import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { API_VERSION, connectionError, type Session, type Project, projects, sessions } from './api';
import { Navigation } from './Navigation';
import { Chat } from './Chat';
import { Notifications } from './push';
import { selectedSession } from './notification';

const SESSION_REFRESH_MS = 15_000;
export function App() {
  const menuButton = useRef<HTMLButtonElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const [menu, setMenu] = useState(false);
  const [help, setHelp] = useState(false);
  const [registered, setRegistered] = useState<Project[]>([]);
  const [active, setActive] = useState<string | null>(() => localStorage.getItem('niles.activeProject'));
  const [items, setItems] = useState<Session[]>([]);
  const [selected, setSelected] = useState<string | undefined>(selectedSession(window.location.hash));
  const [version, setVersion] = useState<number>();
  const [error, setError] = useState('');
  const [loaded, setLoaded] = useState(false);
  const statusRevision = useRef(0);
  useEffect(() => {
    if (!menu) return;
    closeButton.current?.focus();
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') setMenu(false); };
    window.addEventListener('keydown', escape);
    return () => { window.removeEventListener('keydown', escape); menuButton.current?.focus(); };
  }, [menu]);
  useEffect(() => {
    const select = () => { setSelected(selectedSession(window.location.hash)); setHelp(false); setMenu(false); };
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
        const [response, navigation] = await Promise.all([sessions(controller.signal), projects(controller.signal)]);
        if (controller.signal.aborted) return;
        setVersion(response.api_version === API_VERSION ? navigation.api_version : response.api_version);
        setRegistered(navigation.projects);
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
  useEffect(() => {
    const project = items.find(item => item.id === selected)?.project;
    if (project) {
      setActive(project); localStorage.setItem('niles.activeProject', project);
      setSelected(undefined); window.history.replaceState(null, '', window.location.pathname + window.location.search);
    }
  }, [items, selected]);
  const openProject = (name: string) => {
    setActive(name); localStorage.setItem('niles.activeProject', name);
    window.history.replaceState(null, '', window.location.pathname + window.location.search);
    setSelected(undefined); setMenu(false); setHelp(false);
  };
  const current = selected ? items.find(item => item.id === selected) : items.find(item => item.project === active);
  const compatible = version === API_VERSION;
  return <div class="app">
    <header class="toolbar"><button ref={menuButton} aria-label={menu ? 'Close menu' : 'Open menu'} aria-expanded={menu}
      aria-controls="project-menu" onClick={() => setMenu(!menu)}>☰</button><h1>Niles</h1></header>
    {menu && <aside id="project-menu">
      <header><h2>Projects</h2><button ref={closeButton} onClick={() => setMenu(false)}>Close sidebar</button></header>
      {compatible && <Notifications />}
      <Navigation projects={registered} selected={active ?? undefined} openProject={openProject}
        help={() => { setHelp(true); setMenu(false); }} />
      {loaded && compatible && registered.length === 0 && <p class="empty">No registered projects.</p>}
    </aside>}
    {menu && <button class="scrim" aria-label="Dismiss menu" onClick={() => setMenu(false)} />}
    {error && <p class="banner" role="alert">{error}</p>}
    {version !== undefined && !compatible && <p class="banner" role="alert">Update the web app. API version {version} requires a matching release (this app supports {API_VERSION}).</p>}
    {help ? <main class="empty"><h2>Help</h2><p>Open the menu and choose a project to chat with its lead. Workers are grouped by role; Lost means their recorded window is gone.</p>
      <a href="https://github.com/Rhovian/niles/blob/main/docs/remote.md" target="_blank" rel="noopener noreferrer">Remote setup guide</a></main> :
      compatible && current ? <Chat key={current.id} session={current} onStatus={onStatus} /> :
      <main class="empty welcome"><img src="/icon-192.png" alt="Niles logo" width="192" height="192" />
        <p>{version !== undefined && !compatible ? 'Install the matching web release to continue.' :
          !loaded && !error ? 'Connecting…' : active || selected ? 'This project’s lead is not running. Open projects on the sidebar.' : 'Open projects on the sidebar.'}</p></main>}
  </div>;
}
