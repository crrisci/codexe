// Copyright (c) 2026 Francesco Pio Crisci
// SPDX-License-Identifier: Apache-2.0

// Actual CLI startup tests. The executable below simulates an engine, never a model.
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { connect } from 'node:net';
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { isAbsolute, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

const binary = process.env.XENOCODEX_NATIVE_CODEX_BIN;
assert.ok(binary && isAbsolute(binary), 'Set XENOCODEX_NATIVE_CODEX_BIN to the absolute rebuilt fork binary');

const engineFixture = `#!/usr/bin/env python3
import fcntl, json, os, socket, sys, threading, uuid
args = sys.argv[1:]
assert args[0] == 'serve'
root = os.environ['CODEXE_STARTUP_FIXTURE_ROOT']
path = args[args.index('--socket') + 1]
lock_path = os.path.join(os.environ['XDG_STATE_HOME'], 'xenolith', 'engine.lock')
os.makedirs(os.path.dirname(lock_path), mode=0o700, exist_ok=True)
inherited = os.environ.get('XENOLITH_ENGINE_LOCK_FD')
lock_fd = int(inherited) if inherited else os.open(lock_path, os.O_CREAT | os.O_RDWR, 0o600)
fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
assert os.fstat(lock_fd).st_ino == os.stat(lock_path).st_ino
os.ftruncate(lock_fd, 0)
os.write(lock_fd, ('pid %d socket %s\\n' % (os.getpid(), path)).encode())
with open(root + '/spawns.jsonl', 'a') as out:
    out.write(json.dumps({'pid': os.getpid(), 'sid': os.getsid(0), 'inherited_lock': inherited is not None, 'args': args}) + '\\n')
with open(root + '/engine.pid', 'w') as out: out.write(str(os.getpid()))
print('fixture engine startup diagnostic', file=sys.stderr, flush=True)
if os.environ.get('CODEXE_STARTUP_FIXTURE_MODE') == 'exit': sys.exit(17)
server = socket.socket(socket.AF_UNIX)
server.bind(path)
server.listen(32)
sessions = {}
efforts = json.loads(os.environ.get('CODEXE_STARTUP_FIXTURE_EFFORTS', '[]'))
record_lock = threading.Lock()
def connection(peer):
    session = None
    def send(value): peer.sendall((json.dumps(value) + '\\n').encode())
    try:
        with peer, peer.makefile('r') as lines:
            for line in lines:
                request = json.loads(line)
                with record_lock:
                    with open(root + '/requests.jsonl', 'a') as out: out.write(json.dumps(request) + '\\n')
                op = request['op']
                if op == 'describe':
                    send({'ok': True, 'protocol': 1, 'model': 'startup-sim', 'context_window': 32768,
                          'max_output': 8192, 'max_frame': 1048576, 'kvstore': True,
                          'reasoning': {'efforts': efforts, 'history': ['discard', 'preserve_tool_calls'] if efforts else [], 'budget_tokens': bool(efforts)}})
                elif op == 'create':
                    session = uuid.uuid4().hex
                    sessions[session] = 0
                    send({'ok': True, 'session': session, 'marker': 0})
                elif op == 'open':
                    session = request['session']
                    send({'ok': True, 'marker': sessions[session], 'pending': [], 'tokens': 100,
                          'turn_open': False, 'zero_prefill': True, 'resume': 'snapshot'})
                elif op in ('rebuild', 'append'):
                    send({'ok': True, 'marker': sessions[session]})
                elif op == 'checkpoint': send({'ok': True, 'saved': True, 'tokens': 100})
                elif op == 'rewind_cost': send({'ok': True, 'prefill': 0})
                elif op == 'generate':
                    assert request['max_tokens'] == 8192
                    assert request.get('reasoning') is False
                    sessions[session] += 1
                    send({'event': 'start'})
                    send({'event': 'text_delta', 'text': 'STARTUP_READY'})
                    send({'event': 'done', 'stop': 'stop', 'marker': sessions[session],
                          'usage': {'input': 8, 'cache_read': 90, 'output': 2, 'total': 100, 'reasoning': 0, 'replayed': 0},
                          'checkpoint': {'saved': True, 'tokens': 100}})
                elif op == 'cancel': send({'ok': True})
                else: raise AssertionError('unexpected wire operation ' + op)
    except (BrokenPipeError, ConnectionResetError): pass
    except Exception as error:
        with record_lock:
            with open(root + '/fixture-errors.jsonl', 'a') as out: out.write(str(error) + '\\n')
while True:
    peer, _ = server.accept()
    threading.Thread(target=connection, args=(peer,), daemon=True).start()
`;

const flags = Object.entries({
  model_provider: 'xenolith', model_reasoning_effort: 'none', web_search: 'disabled',
  'model_providers.xenolith.name': 'Xenolith',
  'model_providers.xenolith.wire_api': 'xenolith',
  'model_providers.xenolith.requires_openai_auth': false,
  'model_providers.xenolith.supports_websockets': false,
  'model_providers.xenolith.stream_max_retries': 0,
  'model_providers.xenolith.request_max_retries': 0,
}).flatMap(([key, value]) => ['-c', `${key}=${JSON.stringify(value)}`]);

async function contents(path) {
  return readFile(path, 'utf8').catch(error => {
    if (error.code === 'ENOENT') return '';
    throw error;
  });
}

async function records(path) {
  return (await contents(path)).trim().split('\n').filter(Boolean).map(line => JSON.parse(line));
}

function alive(pid) {
  try { process.kill(pid, 0); return true; }
  catch (error) { if (error.code === 'ESRCH') return false; throw error; }
}

async function fixture(t) {
  const parent = resolve('.test-artifacts');
  await mkdir(parent, { recursive: true });
  const root = await mkdtemp(join(parent, 'startup-'));
  const workspace = join(root, 'workspace');
  const home = join(root, 'codex-home');
  const socket = join(root, 'w.sock');
  const engine = join(root, 'fake engine');
  const model = join(root, 'model $(literal argument).gguf');
  await Promise.all([mkdir(workspace), mkdir(home), writeFile(engine, engineFixture, { mode: 0o700 }), writeFile(model, 'fixture; no model weights')]);
  const env = { ...process.env };
  for (const key of Object.keys(env)) {
    if (key.startsWith('XENOLITH_')) delete env[key];
  }
  Object.assign(env, {
    CODEX_HOME: home, HOME: root,
    XDG_STATE_HOME: join(root, 'xdg-state'), XDG_CACHE_HOME: join(root, 'xdg-cache'), XDG_RUNTIME_DIR: join(root, 'runtime'),
    XENOLITH_SOCKET: socket, XENOLITH_BIN: engine, XENOLITH_MODEL: model,
    XENOLITH_STATE_DIR: join(root, 'bindings'),
    XENOLITH_ENGINE_STATE_DIR: join(root, 'engine-state'), XENOLITH_ENGINE_CACHE_DIR: join(root, 'engine-cache'),
    XENOLITH_CONTEXT: '32768', XENOLITH_CACHE_BUDGET: '4GiB',
    CODEXE_STARTUP_FIXTURE_ROOT: root,
  });
  delete env.XENOLITH_ENGINE_LOCK_FD;
  t.after(async () => {
    const pid = Number(await contents(join(root, 'engine.pid')));
    if (Number.isInteger(pid) && pid > 0 && alive(pid)) process.kill(pid, 'SIGTERM');
  });
  let invocation = 0;
  async function invoke(args, overrides = {}) {
    const childEnv = { ...env, ...overrides };
    for (const key of Object.keys(childEnv)) if (childEnv[key] === undefined) delete childEnv[key];
    const child = spawn(binary, args, { cwd: workspace, env: childEnv, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.setEncoding('utf8').on('data', data => { stdout += data; });
    child.stderr.setEncoding('utf8').on('data', data => { stderr += data; });
    const timer = setTimeout(() => child.kill('SIGKILL'), 45000);
    let result;
    try {
      result = await new Promise((resolve, reject) => {
        child.once('error', reject);
        child.once('close', (code, signal) => resolve({ code, signal }));
      });
    } finally { clearTimeout(timer); }
    invocation++;
    await Promise.all([
      writeFile(join(root, `cli-${invocation}.stdout.jsonl`), stdout),
      writeFile(join(root, `cli-${invocation}.stderr.log`), stderr),
    ]);
    return { ...result, stdout, stderr };
  }
  const execute = overrides => invoke(['--no-daemon', 'exec', '--ignore-user-config', '--ignore-rules',
    '--skip-git-repo-check', '--sandbox', 'read-only', '--json', '-C', workspace, ...flags, 'Reply with the startup fixture response.'], overrides);
  return { root, env, socket, engine, model, invoke, execute };
}

async function describe(socket) {
  return new Promise((resolve, reject) => {
    const peer = connect(socket);
    const timer = setTimeout(() => peer.destroy(new Error('describe timed out')), 2000);
    let buffer = '';
    peer.setEncoding('utf8');
    peer.once('connect', () => peer.write('{"op":"describe"}\n'));
    peer.on('data', data => {
      buffer += data;
      if (!buffer.includes('\n')) return;
      clearTimeout(timer);
      peer.destroy();
      try { resolve(JSON.parse(buffer.slice(0, buffer.indexOf('\n')))); }
      catch (error) { reject(error); }
    });
    peer.once('error', error => { clearTimeout(timer); reject(error); });
  });
}

function assertCompleted(result) {
  assert.equal(result.signal, null, result.stderr + result.stdout);
  assert.equal(result.code, 0, result.stderr + result.stdout);
  const events = result.stdout.trim().split('\n').filter(Boolean).map(line => JSON.parse(line));
  assert.equal(events.findLast(event => event.type === 'item.completed' && event.item?.type === 'agent_message')?.item.text, 'STARTUP_READY');
}

test('help, version and config-only commands bypass native service startup', { timeout: 30000 }, async t => {
  const f = await fixture(t);
  for (const args of [['--help'], ['--version'], ['exec', '--help'], ['features', 'list']]) {
    const result = await f.invoke(['--no-daemon', ...flags, ...args]);
    assert.equal(result.code, 0, result.stderr + result.stdout);
  }
  assert.deepEqual(await records(join(f.root, 'spawns.jsonl')), []);
  assert.equal(await contents(join(f.root, 'requests.jsonl')), '');
});

test('CLI discovers by environment, starts once and leaves its engine available after exit', { timeout: 150000 }, async t => {
  const f = await fixture(t);
  assertCompleted(await f.execute());
  const [engine] = await records(join(f.root, 'spawns.jsonl'));
  assert.equal(engine.inherited_lock, true);
  assert.equal(engine.sid, engine.pid, 'Auto-started engine must run in its own Unix session');
  assert.equal(engine.args[1], f.model, 'Model path must be passed literally without a shell');
  for (const [flag, expected] of [
    ['--socket', f.socket], ['--idle-shutdown', '30'], ['--ctx', '32768'],
    ['--state', f.env.XENOLITH_ENGINE_STATE_DIR], ['--cache', f.env.XENOLITH_ENGINE_CACHE_DIR], ['--cache-budget', String(4 * 1024 ** 3)],
  ]) assert.equal(engine.args[engine.args.indexOf(flag) + 1], expected);
  assert.ok(alive(engine.pid), 'Engine must outlive the launching CLI');
  assert.equal((await describe(f.socket)).model, 'startup-sim');
  assertCompleted(await f.execute());
  assertCompleted(await f.execute({ XENOLITH_MODEL: undefined, XENOLITH_BIN: '/must/not/start', XENOLITH_NO_SPAWN: '1' }));
  assert.equal((await records(join(f.root, 'spawns.jsonl'))).length, 1, 'Repeated config loads and later CLIs must reuse the engine');
  const requests = await records(join(f.root, 'requests.jsonl'));
  assert.equal(requests.filter(request => request.op === 'generate').length, 3);
  assert.ok(requests.filter(request => request.op === 'describe').length >= 6, 'Both resolved config and inference must validate describe');
  assert.equal(await contents(join(f.root, 'fixture-errors.jsonl')), '');
  assert.equal((await describe(f.socket)).ok, true);
  console.log(`Native startup artifacts: ${f.root}`);
});

test('CLI connects without a model and preserves an externally started engine', { timeout: 60000 }, async t => {
  const f = await fixture(t);
  const external = spawn(f.engine, ['serve', f.model, '--socket', f.socket, '--state', f.env.XENOLITH_ENGINE_STATE_DIR], {
    env: f.env, stdio: 'ignore', detached: true,
  });
  const exited = new Promise(resolve => external.once('exit', (code, signal) => resolve({ code, signal })));
  t.after(async () => { external.kill('SIGTERM'); await exited; });
  let ready = false;
  for (let attempt = 0; attempt < 50; attempt++) {
    try { await describe(f.socket); ready = true; break; }
    catch { await delay(50); }
  }
  assert.ok(ready, 'External fake engine must become ready');
  assertCompleted(await f.execute({ XENOLITH_MODEL: undefined, XENOLITH_BIN: '/must/not/start', XENOLITH_NO_SPAWN: '1' }));
  assert.ok(alive(external.pid));
  assert.equal((await describe(f.socket)).ok, true);
  const spawns = await records(join(f.root, 'spawns.jsonl'));
  assert.equal(spawns.length, 1);
  assert.equal(spawns[0].inherited_lock, false, 'This service belongs to the external launcher');
  assert.equal(await contents(join(f.root, 'fixture-errors.jsonl')), '');
});

test('CLI reports a failed engine executable with its startup diagnostics', { timeout: 30000 }, async t => {
  const f = await fixture(t);
  const result = await f.execute({ CODEXE_STARTUP_FIXTURE_MODE: 'exit' });
  assert.equal(result.signal, null, result.stderr);
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /exit status: 17/);
  assert.match(result.stderr, /fixture engine startup diagnostic/);
  assert.ok((await records(join(f.root, 'spawns.jsonl'))).length >= 1);
});

test('app-server exposes the discovered single model and engine reasoning choices', { timeout: 45000 }, async t => {
  const f = await fixture(t);
  const child = spawn(binary, ['--no-daemon', ...flags, 'app-server', '--stdio'], {
    cwd: join(f.root, 'workspace'),
    env: { ...f.env, CODEXE_STARTUP_FIXTURE_EFFORTS: JSON.stringify(['low', 'high', 'max']) },
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  const pending = new Map();
  const messages = [];
  let stderr = '', buffer = '', nextId = 0;
  child.stderr.setEncoding('utf8').on('data', data => { stderr += data; });
  child.stdout.setEncoding('utf8').on('data', data => {
    buffer += data;
    for (;;) {
      const end = buffer.indexOf('\n');
      if (end < 0) break;
      const line = buffer.slice(0, end);
      buffer = buffer.slice(end + 1);
      if (!line) continue;
      let message;
      try { message = JSON.parse(line); }
      catch (error) { for (const waiter of pending.values()) waiter.reject(error); return; }
      messages.push(message);
      const waiter = pending.get(message.id);
      if (waiter) { pending.delete(message.id); waiter.resolve(message); }
    }
  });
  const exited = new Promise(resolve => {
    child.once('error', error => { for (const waiter of pending.values()) waiter.reject(error); });
    child.once('close', (code, signal) => {
      for (const waiter of pending.values()) waiter.reject(new Error(`app-server exited ${code}/${signal}: ${stderr}`));
      resolve({ code, signal });
    });
  });
  t.after(async () => {
    child.stdin.end();
    child.kill('SIGTERM');
    const timer = setTimeout(() => child.kill('SIGKILL'), 5000);
    try { await exited; }
    finally { clearTimeout(timer); }
    await Promise.all([
      writeFile(join(f.root, 'app-server.messages.json'), JSON.stringify(messages, null, 2)),
      writeFile(join(f.root, 'app-server.stderr.log'), stderr),
    ]);
  });
  async function request(method, params) {
    const id = ++nextId;
    let timer;
    try {
      return await new Promise((resolve, reject) => {
        timer = setTimeout(() => reject(new Error(`Timed out waiting for ${method}: ${stderr}`)), 15000);
        pending.set(id, { resolve, reject });
        child.stdin.write(JSON.stringify({ id, method, params }) + '\n');
      });
    } finally { clearTimeout(timer); pending.delete(id); }
  }
  const initialized = await request('initialize', {
    clientInfo: { name: 'codexe-native-startup-test', version: '1' },
    capabilities: { experimentalApi: true },
  });
  assert.ok(initialized.result, JSON.stringify(initialized) + stderr);
  child.stdin.write(JSON.stringify({ method: 'initialized' }) + '\n');
  for (let attempt = 0; attempt < 2; attempt++) {
    const response = await request('model/list', { includeHidden: true, limit: 100 });
    assert.ok(response.result, JSON.stringify(response) + stderr);
    assert.equal(response.result.nextCursor, null);
    assert.equal(response.result.data.length, 1, 'The picker must contain only the engine model');
    const [model] = response.result.data;
    assert.equal(model.model, 'startup-sim');
    assert.equal(model.displayName, 'startup-sim (Xenolith)');
    assert.equal(model.hidden, false);
    assert.equal(model.isDefault, true);
    assert.equal(model.defaultReasoningEffort, 'none');
    assert.deepEqual(model.supportedReasoningEfforts.map(option => option.reasoningEffort), ['none', 'low', 'high', 'max']);
    assert.deepEqual(model.inputModalities, ['text']);
    assert.deepEqual(model.serviceTiers, []);
  }
  assert.equal((await records(join(f.root, 'spawns.jsonl'))).length, 1, 'Repeated picker queries must share the engine');
  assert.equal(await contents(join(f.root, 'fixture-errors.jsonl')), '');
});
