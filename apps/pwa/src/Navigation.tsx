import { type Project, startProject, stopProject, closeWorker } from './api';
import { useState } from 'preact/hooks';

const stateGlyphs = { working: '●', starting: '◌', idle: '○', waiting: '⚠', live: '●' } as const;
function StateIcon({ state = 'live', id }: { state?: keyof typeof stateGlyphs; id?: string }) {
  return <span id={id} class={`agent-state state-${state}`} role="img" aria-label={state} title={state}><span aria-hidden="true">{stateGlyphs[state]}</span><span class="sr-only">{state}</span></span>;
}
const liveStates = new Set<Project['state']>(['running', 'starting', 'waiting']);
export function Navigation({ projects, selected, openProject, openWorker, refresh, help, prefix = '' }: {
  prefix?: string; projects: Project[]; selected?: string; openProject: (name: string) => void; openWorker: (project: string, id: string) => void; refresh: () => void; help?: () => void;
}) {
  const [expanded, setExpanded] = useState<string>();
  const [expandedWorker, setExpandedWorker] = useState<string>();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  async function act(action: () => Promise<void>) {
    setPending(true); setError('');
    try { await action(); refresh(); }
    catch (cause) { setError(String(cause)); }
    finally { setPending(false); }
  }
  return <nav aria-label="Projects">
    {error && <p class="banner" role="alert">{error}</p>}
    {projects.map(project => <section key={project.name}>
      <button aria-label={project.name}
        aria-describedby={liveStates.has(project.state) ? `live-${encodeURIComponent(JSON.stringify([prefix, project.name]))}` : undefined}
        class={`session ${selected === project.name ? 'selected' : ''}`}
        aria-expanded={expanded === project.name} aria-controls={`project-${encodeURIComponent(JSON.stringify([prefix, project.name]))}`}
        onClick={() => { setExpanded(expanded === project.name ? undefined : project.name); setExpandedWorker(undefined); }}>
        <span aria-hidden="true">{expanded === project.name ? '▾' : '▸'}</span><strong>{project.name}</strong>
        {liveStates.has(project.state) && <span class="badge project-live" id={`live-${encodeURIComponent(JSON.stringify([prefix, project.name]))}`}><StateIcon state={project.lead_state} /></span>}
      </button>
      {expanded === project.name && <div class="project-details" id={`project-${encodeURIComponent(JSON.stringify([prefix, project.name]))}`}>
        {!liveStates.has(project.state) && <p class="badge">{project.state}</p>}
        <button onClick={() => openProject(project.name)}>Open {project.name} lead chat</button>
        {project.state !== 'missing' && <button disabled={pending} onClick={() => {
          if (project.state === 'not running') void act(() => startProject(project.name, prefix));
          else if (window.confirm(`Stop ${project.name}?`)) void act(() => stopProject(project.name, prefix));
        }}>{project.state === 'not running' ? 'Start' : 'Stop'} {project.name}</button>}
        {[...new Set(project.workers.map(worker => worker.skill))].sort().map(skill => {
          const workers = project.workers.filter(worker => worker.skill === skill);
          return workers.length > 0 && <details key={skill} open>
            <summary>{skill}</summary><ul class="workers">{workers.map(worker => {
              const workerExpanded = expandedWorker === worker.id;
              const panelId = `worker-${encodeURIComponent(JSON.stringify([prefix, project.name, worker.id]))}`;
              return <li key={worker.id}>
                <button class="session" aria-label={worker.id} aria-describedby={`${panelId}-skill${!worker.lost && worker.state ? ` ${panelId}-state` : ''}`}
                  aria-expanded={workerExpanded} aria-controls={panelId}
                  onClick={() => setExpandedWorker(workerExpanded ? undefined : worker.id)}>
                  <span aria-hidden="true">{workerExpanded ? '▾' : '▸'}</span>{worker.id}
                  <span class="badge" id={`${panelId}-skill`}>{skill}{worker.lost ? ' · Lost' : ''}</span>
                  {!worker.lost && worker.state && <StateIcon id={`${panelId}-state`} state={worker.state} />}
                </button>
                {workerExpanded && <div class="worker-actions" id={panelId}>
                  <button onClick={() => openWorker(project.name, worker.id)}>Open {worker.id} chat</button>
                  <button disabled={pending} onClick={() => {
                    if (window.confirm(`Close ${worker.id}?`)) void act(() => closeWorker(project.name, worker.id, prefix));
                  }}>Close {worker.id}</button>
                </div>}
              </li>;
            })}</ul>
          </details>;
        })}
      </div>}
    </section>)}
    {help && <button onClick={help}>Help</button>}
  </nav>;
}
