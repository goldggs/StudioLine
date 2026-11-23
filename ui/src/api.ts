export type Project = { id: string; name: string; created_at: string };

async function request<T>(path: string, options?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, options);
  } catch {
    throw new Error('Cannot reach StudioLine. Check that the local app is running and try again.');
  }
  if (!response.ok) {
    let message = 'Something went wrong. Please try again.';
    try {
      const body: unknown = await response.json();
      if (typeof body === 'object' && body !== null && 'error' in body && typeof body.error === 'string') message = body.error;
    } catch { /* The server did not return a readable error. */ }
    throw new Error(message);
  }
  return response.json() as Promise<T>;
}
export const listProjects = (): Promise<Project[]> => request('/api/projects');
export const createProject = (name: string): Promise<Project> => request('/api/projects', {
  method: 'POST', headers: { 'Content-Type': 'application/json', 'X-StudioLine-Request': '1' }, body: JSON.stringify({ name })
});
export const shutdownApp = (): Promise<{ status: string }> => request('/api/shutdown', {
  method: 'POST', headers: { 'X-StudioLine-Request': '1' }
});

export type StudioStatus = { project_id: string | null; connected: boolean; place_name: string | null; experience_name: string | null; place_id: number | null };
export type PlaceInfo = { name: string; experience_name: string | null; place_id: number; total_children: number; workspace_children: { name: string; class_name: string }[] };
export const activateProject = (project_id: string): Promise<StudioStatus> => request('/api/studio/activate', {
  method: 'POST', headers: { 'Content-Type': 'application/json', 'X-StudioLine-Request': '1' }, body: JSON.stringify({ project_id })
});
export type PluginInstallState = 'installed' | 'missing' | 'update_available';
export const getPluginInstallState = (): Promise<{ state: PluginInstallState }> => request('/api/studio/install');
export const installPlugin = (): Promise<{ installed: boolean }> => request('/api/studio/install', {
  method: 'POST', headers: { 'X-StudioLine-Request': '1' }
});
export const inspectPlace = (project_id: string): Promise<PlaceInfo> => request('/api/studio/inspect', {
  method: 'POST', headers: { 'Content-Type': 'application/json', 'X-StudioLine-Request': '1' }, body: JSON.stringify({ project_id })
});
export const deleteProject = (id: string): Promise<{ deleted: boolean }> => request(`/api/projects/${encodeURIComponent(id)}`, {
  method: 'DELETE', headers: { 'X-StudioLine-Request': '1' }
});
export async function exportData(): Promise<Blob> {
  let response: Response;
  try { response = await fetch('/api/export', { method: 'POST', headers: { 'X-StudioLine-Request': '1' } }); }
  catch { throw new Error('Cannot reach StudioLine. Open the app and try again.'); }
  if (!response.ok) {
    let message = 'Could not export StudioLine data. Try again.';
    try { const body: unknown = await response.json(); if (typeof body === 'object' && body !== null && 'error' in body && typeof body.error === 'string') message = body.error; } catch { /* no readable server error */ }
    throw new Error(message);
  }
  return response.blob();
}

export type TunnelStatus = { state: 'stopped' | 'starting' | 'ready' | 'stopping' | 'error'; url: string | null; error: string | null };
export type AgentSummary = { id: string; name: string | null; role: string | null };
export const startAgentConnection = (project_id: string): Promise<TunnelStatus> => request('/api/agents/start', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-StudioLine-Request': '1' }, body: JSON.stringify({ project_id }) });
export const getAgentConnection = (): Promise<TunnelStatus> => request('/api/agents/status');
export const stopAgentConnection = (): Promise<{ stopped: boolean }> => request('/api/agents/stop', { method: 'POST', headers: { 'X-StudioLine-Request': '1' } });
export const issueAgent = (project_id: string): Promise<{ id: string; token: string }> => request('/api/agents/issue', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-StudioLine-Request': '1' }, body: JSON.stringify({ project_id }) });
export const listAgents = (project_id: string): Promise<AgentSummary[]> => request(`/api/agents?project_id=${encodeURIComponent(project_id)}`);
export const revokeAgent = (id: string): Promise<{ revoked: boolean }> => request(`/api/agents/${encodeURIComponent(id)}`, { method: 'DELETE', headers: { 'X-StudioLine-Request': '1' } });
