import { StrictMode, useEffect, useRef, useState, type FormEvent } from 'react';
import { createRoot } from 'react-dom/client';
import { activateProject, createProject, deleteProject, exportData, inspectPlace, installPlugin, getPluginInstallState, listProjects, shutdownApp, type PlaceInfo, type Project, type StudioStatus, type PluginInstallState } from './api';
import AgentConnections from './AgentConnections';
import './style.css';

function Mark() {
  // This must be inline: the shutdown screen remains open after the local server stops.
  return <svg className="mark" viewBox="0 0 1024 1024" aria-hidden="true" focusable="false">
    <rect width="1024" height="1024" fill="#ffffff"/>
    <polygon points="230,276 568,276 568,364 318,364 318,468 553,468 553,748 230,748 230,660 465,660 465,556 230,556" fill="#202b28"/>
    <polygon points="624,276 712,276 712,660 794,660 794,748 624,748" fill="#5b806b"/>
  </svg>;
}
function Arrow() { return <svg className="arrow" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M5 12h14m-5-5 5 5-5 5"/></svg>; }
function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [project, setProject] = useState<Project | null>(null);
  const [agentModal, setAgentModal] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [modal, setModal] = useState(false);
  const [name, setName] = useState('');
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState('');
  const [studio, setStudio] = useState<StudioStatus>({ project_id: null, connected: false, place_name: null, experience_name: null, place_id: null });
  const [eventConnected, setEventConnected] = useState(false);
  const [opening, setOpening] = useState(false);
  const [studioError, setStudioError] = useState('');
  const [installing, setInstalling] = useState(false);
  const [installed, setInstalled] = useState(false);
  const [pluginState, setPluginState] = useState<PluginInstallState | null>(null);
  const [pluginCheckError, setPluginCheckError] = useState('');
  const [inspecting, setInspecting] = useState(false);
  const [place, setPlace] = useState<PlaceInfo | null>(null);
  const [confirmShutdown, setConfirmShutdown] = useState(false);
  const [shuttingDown, setShuttingDown] = useState(false);
  const [shutdownComplete, setShutdownComplete] = useState(false);
  const [shutdownError, setShutdownError] = useState('');
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState('');
  const [exporting, setExporting] = useState(false);
  const [exportError, setExportError] = useState('');
  async function stopApp() {
    setShuttingDown(true); setShutdownError('');
    try { await shutdownApp(); setShutdownComplete(true); setConfirmShutdown(false); }
    catch (err) { setShutdownError(err instanceof Error ? err.message : 'Could not shut down StudioLine.'); }
    finally { setShuttingDown(false); }
  }
  const input = useRef<HTMLInputElement>(null);
  async function refresh() {
    setLoading(true); setError('');
    try { setProjects(await listProjects()); } catch (e) { setError(e instanceof Error ? e.message : 'Could not load projects.'); }
    finally { setLoading(false); }
  }
  useEffect(() => { void refresh(); }, []);
  async function checkPlugin() {
    setPluginCheckError('');
    try { const result = await getPluginInstallState(); setPluginState(result.state); }
    catch (err) { setPluginState(null); setPluginCheckError(err instanceof Error ? err.message : 'Could not check the Studio plugin.'); }
  }
  useEffect(() => { void checkPlugin(); }, []);
  useEffect(() => { if (modal) input.current?.focus(); }, [modal]);
  useEffect(() => {
    let stopped = false;
    let socket: WebSocket | undefined;
    let retry: ReturnType<typeof setTimeout> | undefined;
    function connect() {
      if (stopped) return;
      const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
      socket = new WebSocket(`${scheme}//${location.host}/api/studio/events`);
      socket.onopen = () => setEventConnected(true);
      socket.onmessage = event => {
        try { setStudio(JSON.parse(String(event.data)) as StudioStatus); } catch { setStudioError('Could not read the Studio connection status.'); }
      };
      socket.onclose = () => { setEventConnected(false); if (!stopped) retry = setTimeout(connect, 2000); };
      socket.onerror = () => socket?.close();
    }
    connect();
    return () => { stopped = true; if (retry) clearTimeout(retry); socket?.close(); };
  }, []);
  async function openProject(item: Project) {
    if (opening) return;
    setOpening(true); setError(''); setStudioError(''); setPlace(null);
    try { await activateProject(item.id); setProject(item); setAgentModal(true); }
    catch (err) { setError(err instanceof Error ? err.message : 'Could not open this project.'); }
    finally { setOpening(false); }
  }
  async function doInstall() {
    setInstalling(true); setStudioError('');
    try { await installPlugin(); setPluginState('installed'); setInstalled(true); }
    catch (err) { setStudioError(err instanceof Error ? err.message : 'Could not install the plugin.'); }
    finally { setInstalling(false); }
  }
  async function removeProject() {
    if (!project || deleting) return;
    setDeleting(true); setDeleteError('');
    try { await deleteProject(project.id); setConfirmDelete(false); setProject(null); await refresh(); }
    catch (err) { setDeleteError(err instanceof Error ? err.message : 'Could not delete this project.'); }
    finally { setDeleting(false); }
  }
  async function downloadExport() {
    if (exporting) return;
    setExporting(true); setExportError('');
    try {
      const data = await exportData(); const url = URL.createObjectURL(data); const link = document.createElement('a');
      link.href = url; link.download = 'StudioLine-export.db'; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (err) { setExportError(err instanceof Error ? err.message : 'Could not export data.'); }
    finally { setExporting(false); }
  }
  async function doInspect() {
    if (!project) return;
    setInspecting(true); setStudioError(''); setPlace(null);
    try { setPlace(await inspectPlace(project.id)); }
    catch (err) { setStudioError(err instanceof Error ? err.message : 'Could not inspect the game.'); }
    finally { setInspecting(false); }
  }
  async function submit(e: FormEvent) {
    e.preventDefault(); if (saving) return; setSaving(true); setFormError('');
    try { const created = await createProject(name); setProjects(current => [created, ...current]); setModal(false); setName(''); await openProject(created); }
    catch (err) { setFormError(err instanceof Error ? err.message : 'Could not create project.'); }
    finally { setSaving(false); }
  }
  if (shutdownComplete) return <main className="shutdown-page"><Mark/><h1>StudioLine is closed</h1><p>The local app has stopped. You can close this tab or open StudioLine again.</p></main>;
  return <div className="shell">
    <header className="header"><div className="brand"><Mark/><span>StudioLine</span></div><span className="header-label">LOCAL WORKSPACE</span><span className="header-state"><i/>Local app running</span><button className="shutdown-button" onClick={() => { setShutdownError(''); setConfirmShutdown(true); }}>Shut down</button></header>
    {project ? <div className="workspace"><aside className="side"><button className="back" onClick={() => { setProject(null); void refresh(); }}><span aria-hidden="true">←</span> All projects</button><div className="side-label">PROJECT</div><div className="side-project"><span className="tile">{project.name.slice(0,1).toUpperCase()}</span><div><strong>{project.name}</strong><small>Local project</small></div></div><div className="side-divider"/><div className="side-label">WORKSPACE</div><div className="nav-active">Overview</div><div className="nav-disabled">Chat <small>Coming later</small></div><div className="nav-disabled">Tasks <small>Coming later</small></div><div className="nav-disabled">Changes <small>Coming later</small></div></aside><main className="project-main"><div className="breadcrumb"><button onClick={() => { setProject(null); void refresh(); }}>Projects</button><span>/</span><strong>{project.name}</strong></div><div className="project-content"><div className="kicker">YOUR PROJECT</div><h1>{project.name}</h1><p className="lead">Your project is saved on this computer.</p><div className="divider"/><h2>Getting started</h2><div className="setup-row"><div className="step">01</div><div className="setup-copy"><strong>Connect Studio</strong><p>{studio.connected && studio.project_id === project.id ? `Connected to ${studio.experience_name || studio.place_name || 'Studio'}.` : pluginState === 'installed' ? 'Open Studio and click Connect in the StudioLine panel.' : 'Install or update the plugin, restart Studio, and click Connect in the StudioLine panel.'}</p></div><span className={`tag ${studio.connected && studio.project_id === project.id ? 'tag-connected' : ''}`}>{!eventConnected ? 'Status unavailable' : studio.connected && studio.project_id === project.id ? 'Connected' : 'Not connected'}</span></div>
          <div className="studio-actions">{pluginState !== null && pluginState !== 'installed' && <button className="secondary" disabled={installing || studio.connected} onClick={() => void doInstall()}>{installing ? 'Installing...' : pluginState === 'update_available' ? 'Update Studio plugin' : 'Install Studio plugin'}</button>}<button className="secondary" disabled={!eventConnected || !studio.connected || studio.project_id !== project.id || inspecting} onClick={() => void doInspect()}>{inspecting ? 'Inspecting...' : 'Inspect game'}</button></div>
          {pluginCheckError && <div className="error-box" role="alert"><strong>Plugin status unavailable</strong><p>{pluginCheckError}</p><button className="secondary" onClick={() => void checkPlugin()}>Check again</button></div>}
          {installed && <p className="studio-note" role="status">Plugin installed. Restart Studio, open a game, and click Connect in the StudioLine panel.</p>}
          {studioError && <div className="error-box" role="alert"><strong>Studio connection</strong><p>{studioError}</p></div>}
          {place && <div className="inspection"><h3>Game inspection</h3><p><strong>{studio.connected && studio.project_id === project.id && studio.place_id === place.place_id ? studio.experience_name || studio.place_name || place.name : place.name}</strong> · Place ID {place.place_id}</p>{studio.experience_name && studio.place_name && studio.experience_name !== studio.place_name && <p>Place: {studio.place_name}</p>}<p>Workspace contains {place.total_children} direct objects. Showing up to 25.</p><ul>{place.workspace_children.map((item, i) => <li key={`${i}-${item.name}`}><span>{item.name}</span><span>{item.class_name}</span></li>)}</ul></div>}<div className="setup-row"><div className="step">02</div><div className="setup-copy"><strong>Connect your agents</strong><p>Start a temporary connection and create a private prompt for each agent.</p><button className="secondary" onClick={() => setAgentModal(true)}>Connect agents</button></div><span className="tag">Available</span></div><div className="setup-row"><div className="step">03</div><div className="setup-copy"><strong>Start working together</strong><p>Chat, tasks, and tools will appear here when they are ready.</p></div><span className="tag">Coming later</span></div><button className="secondary delete-project" onClick={() => { setDeleteError(''); setConfirmDelete(true); }}>Delete project</button><div className="note"><strong>Early development build</strong><span>This is an early connection test. Agent access supports read-only inspection only.</span></div></div></main></div> : <main className="home"><div className="home-top"><div><div className="kicker">YOUR WORKSPACE</div><h1>Projects</h1><p className="lead">Create a project or continue where you left off.</p></div><button className="primary" onClick={() => { setName(''); setFormError(''); setModal(true); }}><span className="plus">+</span> New project</button></div><div className="divider"/><div className="list-head"><h2>Recent projects</h2><span>{projects.length} {projects.length === 1 ? 'project' : 'projects'}</span></div>
      {error && <div className="error-box" role="alert"><strong>Could not open or load projects</strong><p>{error}</p><button className="secondary" onClick={() => void refresh()}>Try again</button></div>}{loading ? <div className="empty">Loading projects...</div> : projects.length === 0 ? <div className="empty-state"><div className="empty-icon"><svg viewBox="0 0 24 24" fill="none"><path d="M3.5 7.5h6l2-2h9v13h-17z"/></svg></div><h3>No projects yet</h3><p>Create a project to get started. Your projects are stored locally.</p><button className="secondary" onClick={() => { setName(''); setFormError(''); setModal(true); }}>Create your first project</button></div> : <div className="project-list">{projects.map(item => <button className="project-row" key={item.id} onClick={() => void openProject(item)} disabled={opening}><span className="tile">{item.name.slice(0,1).toUpperCase()}</span><span className="row-text"><strong>{item.name}</strong><small>Local project</small></span><span className="open-label">Open</span><Arrow/></button>)}</div>}
      <div className="home-footer"><span>LOCAL BY DESIGN</span><p>Projects are saved on this computer. Export includes StudioLine data, not Roblox place files.</p><button className="secondary" disabled={exporting} onClick={() => void downloadExport()}>{exporting ? 'Exporting...' : 'Export StudioLine data'}</button>{exportError && <div className="form-error" role="alert">{exportError}</div>}</div></main>}
    {agentModal && project && <AgentConnections project={project} onClose={() => setAgentModal(false)}/>}
    {confirmDelete && project && <div className="modal-backdrop"><div className="modal" role="dialog" aria-modal="true" aria-labelledby="delete-title" onKeyDown={e => { if (e.key === 'Escape' && !deleting) setConfirmDelete(false); }}><div className="kicker">DELETE PROJECT</div><h2 id="delete-title">Delete {project.name}?</h2><p>This permanently deletes this project from StudioLine. Your Roblox place is not deleted. Disconnect Studio first.</p>{deleteError && <div className="form-error" role="alert">{deleteError}</div>}<div className="modal-actions"><button className="secondary" disabled={deleting} onClick={() => setConfirmDelete(false)}>Cancel</button><button className="danger" disabled={deleting} onClick={() => void removeProject()}>{deleting ? 'Deleting...' : 'Delete project'}</button></div></div></div>}
    {confirmShutdown && <div className="modal-backdrop" onMouseDown={e => { if (e.target === e.currentTarget && !shuttingDown) setConfirmShutdown(false); }}><div className="modal" role="dialog" aria-modal="true" aria-labelledby="shutdown-title" onKeyDown={e => { if (e.key === 'Escape' && !shuttingDown) setConfirmShutdown(false); }}><div className="kicker">LOCAL APP</div><h2 id="shutdown-title">Shut down StudioLine?</h2><p>This stops the local app. You can reopen it when you are ready.</p>{shutdownError && <div className="form-error" role="alert">{shutdownError}</div>}<div className="modal-actions"><button type="button" className="secondary" disabled={shuttingDown} onClick={() => setConfirmShutdown(false)}>Cancel</button><button type="button" className="primary" disabled={shuttingDown} onClick={() => void stopApp()}>{shuttingDown ? 'Stopping...' : 'Shut down'}</button></div></div></div>}
    {modal && <div className="modal-backdrop" onMouseDown={e => { if (e.target === e.currentTarget && !saving) setModal(false); }}><div className="modal" role="dialog" aria-modal="true" aria-labelledby="modal-title" onKeyDown={e => { if (e.key === 'Escape' && !saving) setModal(false); }}><div className="kicker">NEW PROJECT</div><h2 id="modal-title">Create a project</h2><p>Give your project a name. You can connect Studio later.</p><form onSubmit={e => void submit(e)}><label htmlFor="project-name">Project name</label><input ref={input} id="project-name" value={name} onChange={e => setName(e.target.value)} placeholder="For example, Forest Village" maxLength={80} required/><span className="field-help">You can use up to 80 characters.</span>{formError && <div className="form-error" role="alert">{formError}</div>}<div className="modal-actions"><button type="button" className="secondary" disabled={saving} onClick={() => setModal(false)}>Cancel</button><button className="primary" type="submit" disabled={saving}>{saving ? 'Creating...' : 'Create project'}</button></div></form></div></div>}
  </div>;
}

createRoot(document.getElementById('root')!).render(<StrictMode><App/></StrictMode>);
