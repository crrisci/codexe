// Copyright (c) 2026 Francesco Pio Crisci
// SPDX-License-Identifier: Apache-2.0

// Actual Codex process dies after a tool effect and an unacknowledged native append.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:net';
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, readFile, writeFile, readdir } from 'node:fs/promises';
import { resolve, join, isAbsolute } from 'node:path';
import { once } from 'node:events';
const binary=process.env.XENOCODEX_NATIVE_CODEX_BIN;
assert.ok(binary&&isAbsolute(binary),'Set the rebuilt native fork binary');
test('native recovery preserves a completed tool effect without executing it again',{timeout:120000},async t=>{
 await mkdir(resolve('.test-artifacts'),{recursive:true});
 const root=await mkdtemp(resolve('.test-artifacts/native-tool-crash-'));
 const workspace=join(root,'workspace'),home=join(root,'codex-home'),bindings=join(root,'bindings'),socketPath=join(root,'w.sock');
 await Promise.all([workspace,home].map(p=>mkdir(p)));
 const session='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',requests=[],failures=[],sockets=new Set();
 let marker=0,pending=[],generations=0,child,killed=false;
 const server=createServer(socket=>{
  sockets.add(socket);socket.on('close',()=>sockets.delete(socket));socket.on('error',()=>{});
  let buffer='';const send=v=>socket.write(JSON.stringify(v)+'\n');
  socket.setEncoding('utf8').on('data',chunk=>{buffer+=chunk;for(;;){const end=buffer.indexOf('\n');if(end<0)break;const line=buffer.slice(0,end);buffer=buffer.slice(end+1);
   try{const r=JSON.parse(line);requests.push(r);
    if(r.op==='describe')send({ok:true,protocol:1,model:'xenocodex-sim',context_window:65536,max_output:8192,max_frame:1048576,kvstore:true,reasoning:{efforts:[],history:[],budget_tokens:false}});
    else if(r.op==='create')send({ok:true,session,marker:0});
    else if(r.op==='open')send({ok:true,marker,pending,tokens:500,resume:'none',zero_prefill:false,turn_open:false});
    else if(r.op==='rebuild'){pending=[];send({ok:true,marker});}
    else if(r.op==='append'){
     assert.equal(r.role,'tool');assert.equal(r.call_id,17);pending=[];
     assert.ok(!killed,'No duplicate append after recovery');killed=true;
     // Model-side mutation happened; its ACK is intentionally lost.
     child.kill('SIGKILL');
    } else if(r.op==='generate'){
     generations++;assert.ok(generations<=2);send({event:'start'});
     if(generations===1){send({event:'toolcall_start',id:17,name:'exec_command'});send({event:'toolcall_end',id:17,name:'exec_command',arguments:{cmd:"printf 'once\\n' >> effect.txt",login:false,max_output_tokens:1000,yield_time_ms:1000}});pending=[17];}
     else send({event:'text_delta',text:'RECOVERED_WITHOUT_DUPLICATE'});
     marker++;
     send({event:'done',stop:generations===1?'tool_use':'stop',marker,usage:{input:6,cache_read:500,output:3,total:508,reasoning:0,replayed:0},checkpoint:{saved:true,tokens:508}});
    }else throw Error('Unexpected op '+r.op);
   }catch(e){failures.push(e.stack);send({event:'error',code:'invalid_request',error:e.message});}
  }});
 });server.listen(socketPath);await once(server,'listening');
 t.after(async()=>{for(const s of sockets)s.destroy();await new Promise(r=>server.close(r));});
 const cfg={model_provider:'xenolith',model_context_window:65536,model_reasoning_effort:'none',web_search:'disabled',
  'model_providers.xenolith.name':'Xenolith','model_providers.xenolith.base_url':`unix://${socketPath}`,'model_providers.xenolith.wire_api':'xenolith',
  'model_providers.xenolith.requires_openai_auth':false,'model_providers.xenolith.supports_websockets':false,'model_providers.xenolith.stream_max_retries':0,'model_providers.xenolith.request_max_retries':0};
 const flags=Object.entries(cfg).flatMap(([k,v])=>['-c',`${k}=${JSON.stringify(v)}`]);
 async function invoke(resume){
  child=spawn(binary,['--no-daemon','exec','--ignore-user-config','--ignore-rules','--skip-git-repo-check','--sandbox','workspace-write','--json','-C',workspace,'-m','xenocodex-sim',...flags,...(resume?['resume','--last']:[]),resume?'Continue without executing tools.':'Execute the requested shell tool once.'],{cwd:workspace,env:{...process.env,CODEX_HOME:home,XENOLITH_STATE_DIR:bindings},stdio:['ignore','pipe','pipe']});
  let out='',err='';child.stdout.on('data',c=>out+=c);child.stderr.on('data',c=>err+=c);
  const timer=setTimeout(()=>child.kill('SIGTERM'),45000);
  let exit;try{exit=await new Promise((r,j)=>{child.once('error',j);child.once('close',(code,signal)=>r({code,signal}));});}finally{clearTimeout(timer);}
  await writeFile(join(root,resume?'resume.jsonl':'first.jsonl'),out);await writeFile(join(root,resume?'resume.stderr.log':'first.stderr.log'),err);
  return {...exit,out,err};
 }
 try{
  const first=await invoke(false);assert.equal(first.signal,'SIGKILL',first.err);assert.equal(await readFile(join(workspace,'effect.txt'),'utf8'),'once\n');
  const files=(await readdir(bindings)).filter(n=>n.endsWith('.json'));assert.equal(files.length,1);
  const before=JSON.parse(await readFile(join(bindings,files[0]),'utf8'));assert.equal(before.in_flight,true);assert.equal(before.pending.length,1);
  const resumed=await invoke(true);assert.equal(resumed.code,0,resumed.err+failures.join('\n'));assert.ok(resumed.out.includes('RECOVERED_WITHOUT_DUPLICATE'));
  assert.equal(await readFile(join(workspace,'effect.txt'),'utf8'),'once\n');
  const after=JSON.parse(await readFile(join(bindings,files[0]),'utf8'));assert.equal(after.in_flight,false);assert.equal(after.pending.length,0);
  assert.equal(requests.filter(r=>r.op==='rebuild').length,2);assert.equal(requests.filter(r=>r.op==='append').length,1);assert.equal(failures.length,0,failures.join('\n'));
  console.log('Native tool-crash artifacts:',root);
 }finally{await writeFile(join(root,'wire.json'),JSON.stringify(requests,null,2));await writeFile(join(root,'failures.json'),JSON.stringify(failures,null,2));}
});
