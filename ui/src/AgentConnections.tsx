import { useEffect, useState } from 'react';
import { getAgentConnection, issueAgent, listAgents, revokeAgent, startAgentConnection, stopAgentConnection, type AgentSummary, type Project, type TunnelStatus } from './api';

const blank: TunnelStatus = { state: 'stopped', url: null, error: null };
function makePrompt(url: string, token: string): string {
  return `You are a separate StudioLine agent. Use an HTTP-capable tool that can make POST requests. Do not reveal or log the credential to other agents or in shared chat. This connection is temporary; if it stops working, ask the user for an updated prompt.\n\nAddress: ${url}\nCredential: ${token}\n\nFirst register yourself by POSTing JSON {"name":"YOUR NAME","role":"YOUR FREEFORM ROLE"} to /v1/register with the HTTP header Authorization: Bearer <credential> and Content-Type: application/json. Choose your own name and role.\n\nFor this first connection test, POST to /v1/inspect with the same Authorization header to inspect the connected Studio place. Report the result to the user. This build supports only read-only inspection, not edits or chat. If your tools cannot send HTTP POST requests, say so instead of claiming you connected.`;
}
export default function AgentConnections({ project, onClose }: { project: Project; onClose: () => void }) {
  const [status, setStatus] = useState<TunnelStatus>(blank);
  const [agents, setAgents] = useState<AgentSummary[]>([]);
  const [credential, setCredential] = useState<{ id: string; token: string; url: string } | null>(null);
  const [showAddress, setShowAddress] = useState(false);
  const [showPrompt, setShowPrompt] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [copied, setCopied] = useState(false);
  const [confirmStop, setConfirmStop] = useState(false);
  async function refresh() {
    try { const [t, a] = await Promise.all([getAgentConnection(), listAgents(project.id)]); setStatus(t); setAgents(a); }
    catch (err) { setError(err instanceof Error ? err.message : 'Could not check agent connections.'); }
  }
  useEffect(() => { void refresh(); const timer = setInterval(() => void refresh(), 2000); return () => clearInterval(timer); }, [project.id]);
  useEffect(() => { if (credential && status.url !== credential.url) { setCredential(null); setShowPrompt(false); } }, [status.url, credential]);
  async function start() { setBusy(true); setError(''); try { setStatus(await startAgentConnection(project.id)); } catch (e) { setError(e instanceof Error ? e.message : 'Could not start agent connection.'); } finally { setBusy(false); } }
  async function stop() { setBusy(true); setError(''); try { await stopAgentConnection(); setCredential(null); setShowPrompt(false); setConfirmStop(false); await refresh(); } catch (e) { setError(e instanceof Error ? e.message : 'Could not stop agent connection.'); } finally { setBusy(false); } }
  async function create() { if (!status.url) return; setBusy(true); setError(''); try { const issued = await issueAgent(project.id); setCredential({ ...issued, url: status.url }); setShowPrompt(false); setCopied(false); await refresh(); } catch (e) { setError(e instanceof Error ? e.message : 'Could not create an agent connection.'); } finally { setBusy(false); } }
  async function revoke(id: string) { setBusy(true); setError(''); try { await revokeAgent(id); if (credential?.id === id) setCredential(null); await refresh(); } catch (e) { setError(e instanceof Error ? e.message : 'Could not revoke access.'); } finally { setBusy(false); } }
  async function copy() { if (!credential) return; try { await navigator.clipboard.writeText(makePrompt(credential.url, credential.token)); setCopied(true); } catch { setError('Could not copy automatically. Reveal the prompt and copy it manually.'); } }
  return <div className="modal-backdrop"><section className="modal agent-modal" role="dialog" aria-modal="true" aria-labelledby="agent-title" onKeyDown={e => { if (e.key === 'Escape') onClose(); }}>
    <div className="kicker">AGENT CONNECTION</div><h2 id="agent-title">Connect an agent</h2>
    <p>StudioLine handles the tunnel component. The temporary address changes when the tunnel restarts. Stopping the connection revokes its agent credentials. Only share a prompt with the agent it is intended for.</p>
    <div className="agent-status"><strong>Status: {status.state === 'ready' ? 'Ready' : status.state === 'starting' ? 'Starting...' : status.state === 'stopping' ? 'Stopping...' : status.state === 'error' ? 'Unavailable' : 'Not started'}</strong>
      {status.error && <p role="alert">{status.error}</p>}
      {status.url && <div className="secret-row"><label htmlFor="tunnel-address">Tunnel address</label><div><input id="tunnel-address" type={showAddress ? 'text' : 'password'} readOnly value={status.url} /><button className="secondary" onClick={() => setShowAddress(!showAddress)}>{showAddress ? 'Hide' : 'Reveal'}</button></div></div>}
    </div>
    {error && <div className="form-error" role="alert">{error}</div>}
    <div className="agent-actions">{(status.state === 'stopped' || status.state === 'error') ? <button className="primary" disabled={busy} onClick={() => void start()}>Start agent connection</button> : <button className="secondary" disabled={busy || status.state === 'stopping'} onClick={() => setConfirmStop(true)}>Stop connection</button>}
      <button className="secondary" disabled={busy || status.state !== 'ready'} onClick={() => void create()}>Create agent prompt</button></div>
    {confirmStop && <div className="prompt-area"><strong>Stop the connection?</strong><p>All agent credentials will be revoked. Agents will need new prompts to reconnect.</p><div className="agent-actions"><button className="secondary" disabled={busy} onClick={() => setConfirmStop(false)}>Cancel</button><button className="danger" disabled={busy} onClick={() => void stop()}>Stop and revoke access</button></div></div>}
    {credential && <div className="prompt-area"><div className="prompt-heading"><strong>Individual connection prompt</strong><button className="secondary" onClick={() => setShowPrompt(!showPrompt)}>{showPrompt ? 'Hide prompt' : 'Reveal prompt'}</button></div><p>The credential is shown only here. Closing this window clears the prompt; you can revoke it below.</p><pre>{showPrompt ? makePrompt(credential.url, credential.token) : 'Prompt hidden. Reveal or copy it when you are ready.'}</pre><button className="secondary" onClick={() => void copy()}>{copied ? 'Copied' : 'Copy prompt'}</button></div>}
    {agents.length > 0 && <div className="agent-list"><strong>Agent credentials</strong>{agents.map(agent => <div key={agent.id}><span>{agent.name || 'Waiting to register'}{agent.role ? ` · ${agent.role}` : ''}</span><button className="secondary" disabled={busy} onClick={() => void revoke(agent.id)}>Revoke</button></div>)}</div>}
    <div className="modal-actions"><button className="secondary" onClick={onClose}>Close</button></div>
  </section></div>;
}
