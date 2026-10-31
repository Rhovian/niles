import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { API_VERSION, leadBase, workerBase, type Session } from './api';
import { hostPrefix, qualified, useHosts, type Host } from './hosts';
import { Navigation } from './Navigation';
import { Chat } from './Chat';
import { Notifications } from './push';
import { selectedSession } from './notification';

export function App() {
  const menuButton = useRef<HTMLButtonElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const [headerTarget, setHeaderTarget] = useState<HTMLElement | null>(null);
  const [menu, setMenu] = useState(false);
  const [help, setHelp] = useState(false);
  const { hosts, error, refresh, onStatus: hostStatus } = useHosts();
  const [active, setActive] = useState<string | null>(() => localStorage.getItem('niles.activeProject'));
  const [selected, setSelected] = useState<string | undefined>(selectedSession(window.location.hash));
  const [worker, setWorker] = useState<{ host: Host; project: string; id: string }>();
  useEffect(() => {
    if (!menu) return;
    closeButton.current?.focus();
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') setMenu(false); };
    window.addEventListener('keydown', escape);
    return () => { window.removeEventListener('keydown', escape); menuButton.current?.focus(); };
  }, [menu]);
  useEffect(() => {
    const select = () => { setWorker(undefined); setSelected(selectedSession(window.location.hash)); setHelp(false); setMenu(false); };
    window.addEventListener('hashchange', select);
    return () => window.removeEventListener('hashchange', select);
  }, []);
  const items = hosts.flatMap(host => host.state === 'ready' ? host.sessions.map(session => ({ host, session })) : []);
  const current = selected ? items.find(({ host, session }) => qualified(host, session.id) === selected)
    : items.find(({ host, session }) => qualified(host, session.project) === active);
  const currentHostId = current?.host.id;
  const onStatus = useCallback((status: Session) => {
    if (currentHostId !== undefined) hostStatus(currentHostId, status);
  }, [currentHostId, hostStatus]);
  useEffect(() => {
    const match = items.find(({ host, session }) => qualified(host, session.id) === selected);
    const project = match && qualified(match.host, match.session.project);
    if (project) {
      setActive(project); localStorage.setItem('niles.activeProject', project);
      setSelected(undefined); window.history.replaceState(null, '', window.location.pathname + window.location.search);
    }
  }, [hosts, selected]);
  const openProject = (host: Host, project: string) => {
    const name = qualified(host, project);
    setWorker(undefined);
    setActive(name); localStorage.setItem('niles.activeProject', name);
    window.history.replaceState(null, '', window.location.pathname + window.location.search);
    setSelected(undefined); setMenu(false); setHelp(false);
  };
  const workerHost = hosts.find(host => host.id === worker?.host.id);
  const member = workerHost?.state === 'ready' ? workerHost.projects.find(project => project.name === worker?.project)?.workers.find(item => item.id === worker?.id) : undefined;
  const compatible = hosts.some(host => host.state === 'ready');
  const loaded = hosts.some(host => host.state !== 'loading');
  return <div class="app">
    <header class="toolbar"><button ref={menuButton} aria-label={menu ? 'Close menu' : 'Open menu'} aria-expanded={menu}
      aria-controls="project-menu" onClick={() => setMenu(!menu)}>☰</button>
      <div class="pane-heading" ref={setHeaderTarget}>{(help || (!current && (!worker || workerHost?.state !== 'ready'))) && <h1>{help ? 'Help' : 'Niles'}</h1>}</div>
      <img class="brand" src="/icon-32.png" alt="Niles" width="32" height="32" /></header>
    {menu && <aside id="project-menu">
      <header><h2>Projects</h2><button ref={closeButton} onClick={() => setMenu(false)}>Close sidebar</button></header>
      {compatible && <Notifications />}
      {hosts.map(host => <section key={host.id} aria-label={host.name}>
        {host.id && <h3>{host.name}</h3>}
        {host.state === 'ready' ? <Navigation prefix={hostPrefix(host)} projects={host.projects}
          selected={host.projects.find(project => qualified(host, project.name) === active)?.name}
          refresh={refresh} openProject={name => openProject(host, name)}
          openWorker={(project, id) => { openProject(host, project); setWorker({ host, project, id }); }} /> :
          host.state === 'asleep' ? <p>Asleep <button disabled title="Wake is not available yet">Wake {host.name}</button></p> :
          host.state === 'loading' ? <p>Connecting…</p> : null}
        {host.state === 'ready' && host.projects.length === 0 && <p class="empty">No registered projects.</p>}
      </section>)}
      <button onClick={() => { setHelp(true); setMenu(false); }}>Help</button>
    </aside>}
    {menu && <button class="scrim" aria-label="Dismiss menu" onClick={() => setMenu(false)} />}
    {error && <p class="banner" role="alert">{error}</p>}
    {hosts.map(host => host.state === 'incompatible' ? <p key={host.id} class="banner" role="alert">{host.name}: Update the web app. API version {host.version} requires a matching release (this app supports {API_VERSION}).</p> :
      host.state === 'error' ? <p key={host.id} class="banner" role="alert">{host.name}: {host.error}</p> :
      host.state === 'asleep' ? <p key={host.id} class="banner" role="alert">{host.name}: Asleep. Cannot reach Niles. Check your tailnet connection.</p> : null)}
    {help ? <main class="empty"><p>Expand a project in the menu to open its lead chat or chat directly with a worker. Start launches the saved default lead; Stop and Close ask for confirmation. Workers are grouped by skill; Lost means their recorded window is gone.</p>
      <a href="https://github.com/Rhovian/niles/blob/main/docs/remote.md" target="_blank" rel="noopener noreferrer">Remote setup guide</a></main> :
      worker && workerHost?.state === 'ready' ? <Chat key={workerBase(worker.project, worker.id, hostPrefix(worker.host))} base={workerBase(worker.project, worker.id, hostPrefix(worker.host))} id={`${worker.project}/${worker.id}`} title={worker.id} pane={`${worker.host.id ? worker.host.name + ' · ' : ''}${member?.skill ?? 'worker'} · ${worker.id}`} subtitle={member?.model ?? ''} headerTarget={headerTarget} /> :
      current ? <Chat key={qualified(current.host, current.session.id)} base={leadBase(current.session.id, hostPrefix(current.host))} id={current.session.id} title={current.session.project} pane={`${current.host.id ? current.host.name + ' · ' : ''}${current.session.agent.entry_skill}`} subtitle={current.session.agent.model ?? current.session.agent.family} headerTarget={headerTarget} working={!current.session.waiting} onStatus={onStatus} /> :
      <main class="empty welcome"><img src="/icon-192.png" alt="Niles logo" width="192" height="192" />
        <p>{hosts.some(host => host.state === 'incompatible') && !compatible ? 'Install the matching web release to continue.' :
          !loaded && !error ? 'Connecting…' : active || selected ? 'This project’s lead is not running. Open projects on the sidebar.' : 'Open projects on the sidebar.'}</p></main>}
  </div>;
}
