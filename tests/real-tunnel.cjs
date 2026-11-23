const { readFileSync } = require('node:fs');
const { spawn } = require('node:child_process');
const assert = require('node:assert/strict');
const WebSocket = require('../ui/node_modules/ws');
const { join } = require('node:path');
if (!process.env.STUDIOLINE_DATA_DIR || process.env.STUDIOLINE_NETWORK_TEST !== '1') throw Error('Use an isolated test instance and set STUDIOLINE_DATA_DIR and STUDIOLINE_NETWORK_TEST=1.');
const base = 'http://127.0.0.1:38741';
let socket;
let started = false;
const headers = {'Content-Type':'application/json','X-StudioLine-Request':'1'};
const sleep = ms => new Promise(r => setTimeout(r,ms));
async function post(path, obj) { const r=await fetch(base+path,{method:'POST',headers,body:JSON.stringify(obj||{})}); if(!r.ok) throw new Error('local '+path+' status '+r.status+' '+await r.text()); return r.json(); }
async function remote(url, token, path, data) {
  const source = `import json,sys,requests,time
x=json.load(sys.stdin)
h={'Content-Type':'application/json'}
if x['token']: h['Authorization']='Bearer '+x['token']
for attempt in range(8):
 try:
  r=requests.post(x['url']+x['path'],headers=h,json=x['data'],timeout=12)
  print(json.dumps({'status':r.status_code,'body':r.text}));break
 except requests.RequestException:
  if attempt==7: raise
  time.sleep(1)
`;
  return new Promise((resolve,reject)=>{
    const child=spawn('python3',['-c',source]); let stdout='',stderr='';
    child.stdout.on('data',d=>stdout+=d); child.stderr.on('data',d=>stderr+=d);
    child.on('error',reject); child.on('close',status=>status===0?resolve(JSON.parse(stdout)):reject(new Error('Remote request failed for '+path)));
    child.stdin.end(JSON.stringify({url,token,path,data}));
  });
}
(async()=>{
 const initial=await (await fetch(base+'/api/agents/status')).json();
 if(initial.state !== 'stopped') throw Error('Test requires an isolated instance without an existing tunnel.');
 const project=await post('/api/projects',{name:'Network test '+Date.now()}); await post('/api/studio/activate',{project_id:project.id});
 const key=readFileSync(join(process.env.STUDIOLINE_DATA_DIR,'studio-plugin.key'),'utf8');
 socket=new WebSocket('ws://127.0.0.1:38741/api/studio/socket');
 socket.on('message',bytes=>{ const v=JSON.parse(bytes); if(v.type==='command') socket.send(JSON.stringify({type:'result',id:v.id,ok:true,data:{name:'Network test place',place_id:42,workspace_children:[{name:'Baseplate',class_name:'Part'}],total_children:1}})); if(v.type==='shutdown') {socket.send(JSON.stringify({type:'shutdown_ack'}));socket.close();} });
 await new Promise((resolve,reject)=>{socket.once('open',resolve);socket.once('error',reject)});
 socket.send(JSON.stringify({type:'hello',token:key,place_name:'Network test place',place_id:42}));
 for(let i=0;i<30;i++){let s=await (await fetch(base+'/api/studio/status')).json();if(s.connected)break;await sleep(100);if(i===29)throw Error('Plugin not connected');}
 await post('/api/agents/start',{project_id:project.id}); started=true;
 let tunnel; for(let i=0;i<120;i++){tunnel=await (await fetch(base+'/api/agents/status')).json(); if(tunnel.state==='ready') break; if(tunnel.state==='error')throw Error(tunnel.error); await sleep(500);}
 assert.equal(tunnel.state,'ready');
 const issued=await post('/api/agents/issue',{project_id:project.id});
 let noauth; for(let i=0;i<12;i++){try { noauth=await remote(tunnel.url,'','/v1/inspect',{}); if(noauth.status===401)break; } catch {} await sleep(1000);}
 assert.equal(noauth?.status,401,'unauthenticated request must be denied');
 assert.equal((await remote(tunnel.url,issued.token,'/api/shutdown',{})).status,405);
 assert.equal((await remote(tunnel.url,issued.token,'/v1/inspect',{})).status,403);
 assert.equal((await remote(tunnel.url,issued.token,'/v1/register',{name:'Actual HTTP test',role:'Tester'})).status,200);
 const result=await remote(tunnel.url,issued.token,'/v1/inspect',{}); assert.equal(result.status,200); assert.equal(JSON.parse(result.body).workspace_children[0].name,'Baseplate');
 let list=await (await fetch(base+'/api/agents?project_id='+project.id)).json(); assert.equal(list[0].name,'Actual HTTP test');
 const revoked=await fetch(base+'/api/agents/'+issued.id,{method:'DELETE',headers});assert.equal(revoked.status,200);
 assert.equal((await remote(tunnel.url,issued.token,'/v1/inspect',{})).status,401);
 await post('/api/agents/stop',{}); started=false; socket.close();
 console.log('Real Cloudflare tunnel: authentication, registration, Studio inspection, admin isolation and revocation passed.');
})().then(()=>process.exit(0)).catch(async e=>{ console.error(e.stack); if(started) try { await post('/api/agents/stop',{}); } catch {} if(socket) socket.terminate(); process.exit(1); });
