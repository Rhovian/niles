import type { Project } from './api';

const roles = ['worker', 'reviewer', 'security', 'research', 'design'] as const;
export function Navigation({ projects, selected, openProject, help }: {
  projects: Project[]; selected?: string; openProject: (name: string) => void; help: () => void;
}) {
  return <nav aria-label="Projects">
    {projects.map(project => <section key={project.name}>
      <button class={`session ${selected === project.name ? 'selected' : ''}`}
        aria-pressed={selected === project.name} onClick={() => openProject(project.name)}>
        <strong>{project.name}</strong><span class="badge">{project.state}</span>
      </button>
      {roles.map(role => {
        const workers = project.workers.filter(worker => worker.role === role);
        return workers.length > 0 && <details key={role} open>
          <summary>{role}</summary><ul>{workers.map(worker =>
            <li key={worker.id}>{worker.id}{worker.lost && <span class="badge">Lost</span>}</li>)}</ul>
        </details>;
      })}
    </section>)}
    <button onClick={help}>Help</button>
  </nav>;
}
