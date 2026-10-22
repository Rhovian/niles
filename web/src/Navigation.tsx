import { type Project, startProject, stopProject, closeWorker } from './api';
import { useState } from 'preact/hooks';

const liveStates = new Set<Project['state']>(['running', 'starting', 'waiting']);
const roles = ['worker', 'reviewer', 'security', 'research', 'design'] as const;
export function Navigation({ projects, selected, openProject, openWorker, refresh, help }: {
  projects: Project[]; selected?: string; openProject: (name: string) => void; openWorker: (project: string, id: string) => void; refresh: () => void; help: () => void;
}) {
  const [expanded, setExpanded] = useState<string>();
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
        aria-describedby={liveStates.has(project.state) ? `live-${encodeURIComponent(project.name)}` : undefined}
        class={`session ${selected === project.name ? 'selected' : ''}`}
        aria-expanded={expanded === project.name} aria-controls={`project-${encodeURIComponent(project.name)}`}
        onClick={() => setExpanded(expanded === project.name ? undefined : project.name)}>
        <span aria-hidden="true">{expanded === project.name ? '▾' : '▸'}</span><strong>{project.name}</strong>
        {liveStates.has(project.state) && <span class="badge project-live" id={`live-${encodeURIComponent(project.name)}`}>Live</span>}
      </button>
      {expanded === project.name && <div class="project-details" id={`project-${encodeURIComponent(project.name)}`}>
        <p class="badge">{project.state}</p>
        <button onClick={() => openProject(project.name)}>Open {project.name} lead chat</button>
        {project.state !== 'missing' && <button disabled={pending} onClick={() => {
          if (project.state === 'not running') void act(() => startProject(project.name));
          else if (window.confirm(`Stop ${project.name}?`)) void act(() => stopProject(project.name));
        }}>{project.state === 'not running' ? 'Start' : 'Stop'} {project.name}</button>}
        {roles.map(role => {
          const workers = project.workers.filter(worker => worker.role === role);
          return workers.length > 0 && <details key={role} open>
            <summary>{role}</summary><ul>{workers.map(worker =>
              <li key={worker.id}><button onClick={() => openWorker(project.name, worker.id)}>{worker.id}</button>{worker.lost && <span class="badge">Lost</span>}<button disabled={pending} onClick={() => {
                if (window.confirm(`Close ${worker.id}?`)) void act(() => closeWorker(project.name, worker.id));
              }}>Close {worker.id}</button></li>)}</ul>
          </details>;
        })}
      </div>}
    </section>)}
    <button onClick={help}>Help</button>
  </nav>;
}
