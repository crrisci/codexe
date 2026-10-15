// Copyright (c) 2026 CodeXe contributors. Licensed under Apache-2.0.
// Explicit, real-weight smoke test. Intentionally excluded from test-*.mjs / CI.
import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { createConnection } from 'node:net';
import { randomBytes, createHash } from 'node:crypto';
import { access, chmod, mkdir, mkdtemp, readFile, readdir, realpath, stat, statfs, writeFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const run = promisify(execFile);
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const GiB = 1024 ** 3;
const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
let root, socketPath, env, initialOwner, probe, wire, activeChild, interrupted;
for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => {
  interrupted = new Error(`Smoke interrupted by ${signal}`);
  process.exitCode = signal === 'SIGINT' ? 130 : 143;
  try { activeChild?.kill('SIGKILL'); } catch {}
  wire?.close();
});
function checkInterrupted() { if (interrupted) throw interrupted; }
const report = { started: new Date().toISOString(), cli: [], snapshot: { verified: false }, engine_pids: [] };

async function requiredFile(name, executable = false) {
  const path = process.env[name];
  assert.ok(path && isAbsolute(path), `${name} must be an explicit absolute existing path`);
  assert.ok((await stat(path)).isFile(), `${name} must be a file`);
  await access(path, executable ? constants.X_OK : constants.R_OK);
  return realpath(path);
}
function positive(name, fallback) {
  const value = process.env[name] ?? String(fallback);
  assert.match(value, /^[0-9]+$/, `${name} must be a positive integer`);
  const n = Number(value);
  assert.ok(Number.isSafeInteger(n) && n > 0, `${name} must be a positive safe integer`);
  return n;
}
function cacheBytes(value) {
  const match = /^(\d+)(B|kB|MB|GB|KiB|MiB|GiB)?$/.exec(value);
  assert.ok(match, 'XENOLITH_CACHE_BUDGET must be a byte count or supported size, e.g. 4GiB');
  const factor = { B: 1, kB: 1000, MB: 1e6, GB: 1e9, KiB: 1024, MiB: 1024 ** 2, GiB }[match[2] ?? 'B'];
  const n = Number(match[1]) * factor;
  assert.ok(Number.isSafeInteger(n) && n > 0, 'Cache budget must be positive');
  return n;
}
async function processInfo(pid) {
  try {
    const [exe, cmdline, status] = await Promise.all([
      realpath(`/proc/${pid}/exe`), readFile(`/proc/${pid}/cmdline`, 'utf8'), readFile(`/proc/${pid}/stat`, 'utf8'),
    ]);
    const tail = status.slice(status.lastIndexOf(')') + 2).split(' ');
    return { pid: Number(pid), exe, args: cmdline.split('\0').filter(Boolean), start: tail[19], state: tail[0] };
  } catch (error) {
    if (['ENOENT', 'ESRCH', 'EACCES'].includes(error.code)) return null;
    throw error;
  }
}
function usesSocket(info) {
  return info?.exe === env.XENOLITH_BIN && info.args.some((arg, i) => arg === '--socket' && info.args[i + 1] === socketPath);
}
async function ownedEngine() {
  const matches = (await Promise.all((await readdir('/proc')).filter(name => /^\d+$/.test(name)).map(processInfo))).filter(usesSocket);
  assert.ok(matches.length <= 1, `Multiple engines claim the private test socket: ${JSON.stringify(matches)}`);
  return matches[0];
}
async function ownerReady() {
  for (let attempt = 0; attempt < 50; attempt++) {
    checkInterrupted();
    const owner = await ownedEngine();
    if (owner) {
      report.engine_pids.push(owner);
      return owner;
    }
    await sleep(100);
  }
  throw new Error('Cannot identify the auto-started engine by executable and unique socket; refusing to signal unknown processes');
}
async function stopOwned(owner) {
  if (!owner) return;
  const same = async () => {
    const current = await processInfo(owner.pid);
    return current && current.start === owner.start && usesSocket(current) && current.state !== 'Z';
  };
  if (!(await same())) return;
  process.kill(owner.pid, 'SIGTERM');
  for (let attempt = 0; attempt < 100; attempt++) {
    if (!(await same())) return;
    await sleep(100);
  }
  if (await same()) process.kill(owner.pid, 'SIGKILL');
  for (let attempt = 0; attempt < 50; attempt++) {
    if (!(await same())) return;
    await sleep(100);
  }
  throw new Error(`Owned engine ${owner.pid} did not stop`);
}
async function revision(cwd) {
  try { return (await run('git', ['-C', cwd, 'rev-parse', 'HEAD'])).stdout.trim(); }
  catch { return null; }
}
async function rollouts(home) {
  const base = join(home, 'sessions');
  const files = (await readdir(base, { recursive: true })).filter(name => name.endsWith('.jsonl'));
  return (await Promise.all(files.map(name => readFile(join(base, name), 'utf8'))))
    .flatMap(text => text.trim().split('\n').filter(Boolean).map(line => JSON.parse(line)));
}
async function invoke(label, resume, prompt, expected, requireTool = false) {
  checkInterrupted();
  const flags = { model_provider: 'xenolith', model_reasoning_effort: 'low', show_raw_agent_reasoning: true, web_search: 'disabled',
    'model_providers.xenolith.name': 'Xenolith', 'model_providers.xenolith.wire_api': 'xenolith',
    'model_providers.xenolith.requires_openai_auth': false, 'model_providers.xenolith.supports_websockets': false,
    'model_providers.xenolith.stream_max_retries': 0, 'model_providers.xenolith.request_max_retries': 0 };
  const child = spawn(env.XENOCODEX_NATIVE_CODEX_BIN, ['--no-daemon', 'exec', '--ignore-user-config', '--ignore-rules',
    '--skip-git-repo-check', '--sandbox', 'read-only', '--json', '-C', join(root, 'workspace'),
    ...Object.entries(flags).flatMap(([key, value]) => ['-c', `${key}=${JSON.stringify(value)}`]),
    ...(resume ? ['resume', '--last'] : []), prompt], { cwd: join(root, 'workspace'), env, stdio: ['ignore', 'pipe', 'pipe'] });
  activeChild = child;
  let stdout = '', stderr = '';
  child.stdout.setEncoding('utf8').on('data', chunk => { stdout += chunk; });
  child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
  // Includes startup, inference and shell execution; logs remain on failure.
  const timer = setTimeout(() => child.kill('SIGKILL'), 600000);
  let result;
  try {
    result = await new Promise((resolve, reject) => { child.once('error', reject); child.once('close', (code, signal) => resolve({ code, signal })); });
  } finally {
    clearTimeout(timer);
    activeChild = null;
    await Promise.all([writeFile(join(root, `${label}.jsonl`), stdout), writeFile(join(root, `${label}.stderr.log`), stderr)]);
  }
  report.cli.push({ label, ...result });
  assert.equal(result.code, 0, `${label} failed (${result.signal ?? result.code}); see ${root}`);
  const events = stdout.trim().split('\n').filter(Boolean).map(line => JSON.parse(line));
  assert.ok(events.some(event => event.type === 'turn.completed'), `${label} must complete an actual turn`);
  assert.equal(events.findLast(event => event.type === 'item.completed' && event.item?.type === 'agent_message')?.item.text, expected,
    `${label}: final answer must exactly match the nonce from the file`);
  if (requireTool) assert.ok(events.some(event => event.type === 'item.completed' && event.item?.type === 'command_execution'
    && event.item.exit_code === 0 && event.item.aggregated_output?.includes(expected)), 'Actual successful shell execution must return the nonce');
  const transcript = await rollouts(env.CODEX_HOME);
  assert.ok(transcript.some(event => event.type === 'response_item' && event.payload?.type === 'reasoning'
    && event.payload.content?.some(part => typeof part.text === 'string' && part.text.trim())), 'Actual raw reasoning must be stored as nonempty reasoning content');
}

// A separate native session proves snapshot loading without touching CodeXe's binding.
class Wire {
  constructor(socket) {
    this.socket = socket;
    this.lines = [];
    this.pending = [];
    this.buffer = '';
    this.failure = null;
    socket.setEncoding('utf8').on('data', chunk => {
      this.buffer += chunk;
      if (this.buffer.length > 8 * 1024 * 1024) return socket.destroy(new Error('Oversized native frame'));
      for (;;) {
        const end = this.buffer.indexOf('\n');
        if (end < 0) break;
        const line = this.buffer.slice(0, end); this.buffer = this.buffer.slice(end + 1);
        if (this.pending.length) this.pending.shift().resolve(line); else this.lines.push(line);
      }
    });
    const fail = error => {
      this.failure ??= error;
      for (const pending of this.pending.splice(0)) pending.reject(error);
    };
    socket.on('error', fail).on('close', () => fail(new Error('Native socket closed')));
  }
  static async connect() {
    checkInterrupted();
    const socket = createConnection(socketPath);
    await new Promise((resolve, reject) => { socket.once('connect', resolve); socket.once('error', reject); });
    return new Wire(socket);
  }
  async receive() {
    if (this.failure) throw this.failure;
    let timer;
    const line = await Promise.race([
      this.lines.length ? Promise.resolve(this.lines.shift()) : new Promise((resolve, reject) => this.pending.push({ resolve, reject })),
      new Promise((_, reject) => { timer = setTimeout(() => { this.socket.destroy(); reject(new Error('Native probe timed out')); }, 300000); }),
    ]).finally(() => clearTimeout(timer));
    const response = JSON.parse(line);
    await writeFile(join(root, 'probe-wire.jsonl'), JSON.stringify({ direction: 'response', response }) + '\n', { flag: 'a' });
    if (response.ok === false || response.event === 'error') throw new Error(`Native probe rejected: ${line}`);
    return response;
  }
  async send(request) {
    checkInterrupted();
    await writeFile(join(root, 'probe-wire.jsonl'), JSON.stringify({ direction: 'request', request }) + '\n', { flag: 'a' });
    this.socket.write(JSON.stringify(request) + '\n');
  }
  async request(request) { await this.send(request); const response = await this.receive(); assert.equal(response.ok, true); return response; }
  async generate() {
    await this.send({ op: 'generate', reasoning: false, max_tokens: 8 });
    for (;;) { const event = await this.receive(); if (event.event === 'done') return event; }
  }
  close() { this.socket.destroy(); }
}

try {
  assert.equal(process.platform, 'linux', 'Real-engine smoke supports Linux and WSL only');
  const [engine, model, codex] = await Promise.all([requiredFile('XENOLITH_BIN', true), requiredFile('XENOLITH_MODEL'), requiredFile('XENOCODEX_NATIVE_CODEX_BIN', true)]);
  const memory = /^MemAvailable:\s+(\d+)\s+kB$/m.exec(await readFile('/proc/meminfo', 'utf8'));
  assert.ok(memory && Number(memory[1]) * 1024 >= 21 * GiB, 'Refusing model allocation: MemAvailable must be at least 21 GiB');
  try {
    const found = await run('pgrep', ['-x', 'xenolith']);
    assert.fail(`An existing Xenolith engine is running (PIDs ${found.stdout.trim()}); stop it before the smoke test`);
  } catch (error) { if (error.code !== 1) throw error; }
  for (const pid of (await readdir('/proc')).filter(name => /^\d+$/.test(name))) {
    const info = await processInfo(pid);
    assert.ok(info?.exe !== engine, `An existing engine already owns weights: PID ${pid}`);
  }
  const globalState = process.env.XDG_STATE_HOME && isAbsolute(process.env.XDG_STATE_HOME)
    ? process.env.XDG_STATE_HOME : join(process.env.HOME, '.local/state');
  const engineLock = join(globalState, 'xenolith/engine.lock');
  // Check the real shared lock without creating/replacing it. The engine also
  // acquires this same lock atomically during auto-start, closing the race.
  await run('python3', ['-c', 'import os,sys,fcntl\np=sys.argv[1]\nif os.path.exists(p):\n f=os.open(p,os.O_RDONLY)\n try: fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)\n except BlockingIOError: sys.exit("Existing Xenolith weight owner holds "+p)\n os.close(f)', engineLock]);
  const parent = join(repo, '.test-artifacts');
  await mkdir(parent, { recursive: true });
  root = await mkdtemp(join(parent, 'model-smoke-'));
  await chmod(root, 0o700);
  // Use a short private socket directory even when the checkout path is long.
  const socketDir = await mkdtemp('/tmp/codexe-smoke-');
  await chmod(socketDir, 0o700);
  socketPath = join(socketDir, 'wire.sock');
  const context = positive('XENOLITH_CONTEXT', 16384);
  const cacheBudget = process.env.XENOLITH_CACHE_BUDGET ?? '4GiB';
  const disk = await statfs(root);
  const free = disk.bavail * disk.bsize;
  assert.ok(free >= cacheBytes(cacheBudget) + GiB, 'Insufficient artifact/cache disk: require cache budget plus 1 GiB free');
  await Promise.all(['workspace', 'home', 'bindings', 'engine-state', 'engine-cache'].map(name => mkdir(join(root, name), { mode: 0o700 })));
  env = { ...process.env, XENOLITH_BIN: engine, XENOLITH_MODEL: model, XENOCODEX_NATIVE_CODEX_BIN: codex,
    XENOLITH_SOCKET: socketPath, XENOLITH_CONTEXT: String(context), XENOLITH_CACHE_BUDGET: cacheBudget,
    XENOLITH_ENGINE_STATE_DIR: join(root, 'engine-state'), XENOLITH_ENGINE_CACHE_DIR: join(root, 'engine-cache'),
    XENOLITH_STATE_DIR: join(root, 'bindings'), CODEX_HOME: join(root, 'home'),
    XENOLITH_MAX_OUTPUT_TOKENS: '1024', XENOLITH_REASONING_BUDGET: '128', XENOLITH_REASONING_HISTORY: 'preserve_tool_calls', XENOLITH_IDLE_SHUTDOWN: '30' };
  delete env.XENOLITH_NO_SPAWN;
  report.config = { engine, model, codex, context, cache_budget: cacheBudget, socket: socketPath, engine_lock: engineLock,
    memory_available_bytes: Number(memory[1]) * 1024, disk_available_bytes: free, max_output: 1024, reasoning: { effort: 'low', budget_tokens: 128, history: 'preserve_tool_calls' } };
  report.commits = { codexe: await revision(repo), engine: await revision(dirname(engine)) };
  try { report.protocol_sha256 = createHash('sha256').update(await readFile(join(dirname(engine), 'PROTOCOL.md'))).digest('hex'); } catch {}
  await writeFile(join(root, 'report.json'), JSON.stringify(report, null, 2));
  const nonce = `SMOKE_${randomBytes(16).toString('hex')}`;
  await writeFile(join(root, 'workspace/nonce.txt'), nonce + '\n', { mode: 0o600 });
  console.log(`Real-model smoke artifacts: ${root}`);
  await invoke('first', false, 'Use exec_command to run cat nonce.txt. Then return only its exact contents without quotes or formatting.', nonce, true);
  initialOwner = await ownerReady();
  await invoke('resume-live', true, 'Return the exact contents you read from nonce.txt again, with no other text.', nonce);
  const records = (await readdir(env.XENOLITH_STATE_DIR)).filter(name => name.endsWith('.json'));
  assert.equal(records.length, 1, 'All CLI processes must resume the same bound thread');
  report.binding_before_restart = JSON.parse(await readFile(join(env.XENOLITH_STATE_DIR, records[0]), 'utf8'));
  assert.equal(report.binding_before_restart.in_flight, false);
  wire = await Wire.connect();
  report.describe = await wire.request({ op: 'describe' });
  const created = await wire.request({ op: 'create', system: 'Reply with OK.', tools: [] });
  probe = created.session;
  await wire.request({ op: 'append', role: 'user', text: 'Say OK.' });
  const before = await wire.generate();
  const checkpoint = await wire.request({ op: 'checkpoint' });
  report.snapshot = { verified: false, session: probe, before, checkpoint };
  assert.ok(checkpoint.saved || before.checkpoint?.saved, `No usable probe checkpoint: ${JSON.stringify(report.snapshot)}. KV restoration is unverified.`);
  wire.close(); wire = null;
  await stopOwned(initialOwner);
  await invoke('resume-restarted', true, 'Return the exact nonce.txt contents from this conversation once more, with no other text.', nonce);
  const restarted = await ownerReady();
  assert.notEqual(`${restarted.pid}:${restarted.start}`, `${initialOwner.pid}:${initialOwner.start}`, 'CodeXe must auto-start a different engine process');
  assert.equal((await readdir(env.XENOLITH_STATE_DIR)).filter(name => name.endsWith('.json')).length, 1, 'Restart must retain one CodeXe conversation binding');
  const afterBinding = JSON.parse(await readFile(join(env.XENOLITH_STATE_DIR, records[0]), 'utf8'));
  assert.equal(afterBinding.session, report.binding_before_restart.session, 'Restart must reopen the original native session');
  assert.equal(afterBinding.in_flight, false);
  assert.ok(afterBinding.epoch > report.binding_before_restart.epoch, 'Restarted resume must commit a newer turn on the same binding');
  wire = await Wire.connect();
  report.snapshot.open = await wire.request({ op: 'open', session: probe });
  await wire.request({ op: 'append', role: 'user', text: 'Say OK again.' });
  report.snapshot.after = await wire.generate();
  assert.equal(report.snapshot.after.resume?.loaded, true,
    `Actual done.resume.loaded:true was not reported; CLI resume succeeded but snapshot restoration remains unverified: ${JSON.stringify(report.snapshot.after)}`);
  checkInterrupted();
  report.snapshot.verified = true;
  report.result = 'passed';
  console.log('PASS: real CLI shell output, raw reasoning, live and restarted thread resume; separate probe confirms actual KV snapshot loading.');
} catch (error) {
  report.result = 'failed';
  report.error = error.stack;
  console.error(error.stack);
  process.exitCode = 1;
} finally {
  wire?.close();
  if (env && socketPath) {
    try { await stopOwned(await ownedEngine()); }
    catch (error) { report.cleanup_error = error.stack; process.exitCode = 1; console.error(`Owned-engine cleanup failed: ${error.message}`); }
  }
  if (root) {
    report.finished = new Date().toISOString();
    await writeFile(join(root, 'report.json'), JSON.stringify(report, null, 2));
    console.log(`Preserved smoke evidence: ${root}`);
  }
}
