import { afterEach, describe, expect, it, vi } from 'vitest';
import { createProject, listProjects, shutdownApp } from './api';

afterEach(() => vi.unstubAllGlobals());
describe('project API', () => {
  it('lists projects', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify([{ id: '1', name: 'Village', created_at: 'now' }]), { status: 200 })));
    expect(await listProjects()).toEqual([{ id: '1', name: 'Village', created_at: 'now' }]);
  });
  it('sends the local request marker when creating a project', async () => {
    const fetcher = vi.fn().mockResolvedValue(new Response(JSON.stringify({ id: '1', name: 'Village', created_at: 'now' }), { status: 201 }));
    vi.stubGlobal('fetch', fetcher);
    await createProject('Village');
    expect(fetcher).toHaveBeenCalledWith('/api/projects', expect.objectContaining({ method: 'POST', headers: expect.objectContaining({ 'X-StudioLine-Request': '1' }), body: JSON.stringify({ name: 'Village' }) }));
  });
  it('requests shutdown with the local request marker', async () => {
    const fetcher = vi.fn().mockResolvedValue(new Response(JSON.stringify({ status: 'shutting_down' }), { status: 200 }));
    vi.stubGlobal('fetch', fetcher);
    await expect(shutdownApp()).resolves.toEqual({ status: 'shutting_down' });
    expect(fetcher).toHaveBeenCalledWith('/api/shutdown', expect.objectContaining({ method: 'POST', headers: expect.objectContaining({ 'X-StudioLine-Request': '1' }) }));
  });
  it('shows a useful error when the local app cannot be reached', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Network failed')));
    await expect(listProjects()).rejects.toThrow('Cannot reach StudioLine');
  });
  it('reports a server validation message without claiming success', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: 'Enter a project name.' }), { status: 400 })));
    await expect(createProject(' ')).rejects.toThrow('Enter a project name.');
  });
});
