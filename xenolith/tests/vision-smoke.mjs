// Copyright (c) 2026 Francesco Pio Crisci. SPDX-License-Identifier: Apache-2.0
// Opt-in real weights only. Excluded from test-*.mjs and ordinary CI.
import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { constants, createReadStream, createWriteStream } from 'node:fs';
import { access, chmod, mkdir, mkdtemp, readFile, readdir, realpath, stat, statfs, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { release, cpus } from 'node:os';
import { cliArgs, connectWire, imageFixture, phaseEvidence, runClient, sleep, visionEnv, wireObserver } from './helpers/vision-smoke.mjs';

const execute = promisify(execFile), GiB = 1024 ** 3;
const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const recoveryMode = process.argv.includes('--recovery');
assert.ok(process.argv.slice(2).every(arg => arg === '--recovery'), 'Only --recovery is supported');
const control = new AbortController();
for (const name of ['SIGINT', 'SIGTERM']) process.once(name, () => {
  process.exitCode = name === 'SIGINT' ? 130 : 143;control.abort(new Error(`Vision smoke interrupted by ${name}`));
});
const report = { mode: recoveryMode ? 'real_vision_recovery' : 'real_vision_smoke', status: 'running', started: new Date().toISOString(), cases: [],
  engine_pids: [], snapshot_probe: { verified: false }, scope: 'Synthetic square colors; not general visual quality or CLI KV certification.' };
let root, socketDir, engine, engineLog, observer, wire, trace, enginePath, engineSocket;
const frames = [];
const check = () => control.signal.throwIfAborted();
async function hashFile(path) {
  const hash = createHash('sha256');for await (const bytes of createReadStream(path)) hash.update(bytes);return hash.digest('hex');
}
async function required(name, executable = false) {
  const path = process.env[name];assert.ok(path && isAbsolute(path), `${name} must be an explicit absolute file path`);
  assert.ok((await stat(path)).isFile(), `${name} must be a file`);
  await access(path, executable ? constants.X_OK : constants.R_OK);return realpath(path);
}
async function source(cwd) {
  try {
    return { commit: (await execute('git', ['-C', cwd, 'rev-parse', 'HEAD'])).stdout.trim(),
      dirty: !!(await execute('git', ['-C', cwd, 'status', '--porcelain', '--untracked-files=no'])).stdout.trim() };
  } catch { return { commit: null, dirty: null }; }
}
async function flush() {
  if (root) await writeFile(join(root, 'report.json'), JSON.stringify(report, null, 2));
}
async function stopEngine() {
  if (!engine) return;
  const child = engine;
  if (child.exitCode === null && child.signalCode === null) {
    const closed = new Promise(resolve => child.once('close', resolve));
    // This is the exact child spawned here, still unreaped; never search/kill by name.
    child.kill('SIGTERM');
    const kill = setTimeout(() => child.kill('SIGKILL'), 10000);
    try { await closed; } finally { clearTimeout(kill); }
  }
  if (engineLog && !engineLog.writableFinished) await new Promise((resolve, reject) => {
    engineLog.once('error', reject);engineLog.end(resolve);
  });
  engine = null;engineLog = null;
}
async function startEngine(label, config) {
  check();
  engineLog = createWriteStream(join(root, `${label}-engine.log`), { mode: 0o600 });
  engine = spawn(config.engine, ['serve', config.model, '--mmproj', config.projector, '--ctx', '65536',
    '--socket', engineSocket, '--state', join(root, 'engine-state'), '--cache', join(root, 'engine-cache'), '--cache-budget', String(4 * GiB)],
    { stdio: ['ignore', 'pipe', 'pipe'] });
  let failure;engine.on('error', error => { failure = error; });
  engine.stdout.pipe(engineLog, { end: false });engine.stderr.pipe(engineLog, { end: false });
  report.engine_pids.push({ pid: engine.pid, label, started: new Date().toISOString() });await flush();
  const deadline = Date.now() + 300000;
  while (Date.now() < deadline) {
    check();if (failure) throw failure;
    assert.equal(engine.exitCode, null, `${label}: engine exited; see engine log`);
    assert.equal(engine.signalCode, null, `${label}: engine was signalled`);
    let connection;
    try {
      connection = await connectWire(engineSocket, () => {}, control.signal);
      const info = await connection.request({ op: 'describe' }, 1000);
      assert.equal(info.protocol, 2, 'Vision smoke requires native protocol v2');
      assert.ok(info.vision?.formats.includes('png') && info.vision.budgets.includes(info.vision.default_budget), 'Advertised PNG vision/projector capabilities required');
      assert.equal(info.kvstore, true, 'Native snapshot probe requires KV storage');
      assert.ok(info.context_window >= 65536);
      if (report.describe) assert.deepEqual(info, report.describe, 'Restart must retain capabilities');
      report.describe = info;return;
    } catch (error) {
      if (!['ENOENT', 'ECONNREFUSED'].includes(error.code) && error.message !== 'Native probe timeout') throw error;
    } finally { connection?.close(); }
    await sleep(250);
  }
  throw new Error('Vision engine readiness timeout');
}
async function binding(directory, thread) {
  const files = (await readdir(directory)).filter(name => name.endsWith('.json'));
  if (!thread) assert.equal(files.length, 1, 'Each private conversation must have exactly one native binding');
  else assert.ok(files.includes(`${thread}.json`), 'Expected thread binding must exist');
  const value = JSON.parse(await readFile(join(directory, thread ? `${thread}.json` : files[0]), 'utf8'));
  assert.equal(value.in_flight, false, 'Conversation must be durably committed');return value;
}
async function invoke(label, config, fixture, { resume = false, attachment = false, tool = false, prompt } = {}) {
  check();observer.phase(label);
  const item = { label, status: 'running' };report.cases.push(item);await flush();
  const outcome = await runClient(config.codex, cliArgs({ workspace: config.workspace, resume, image: attachment ? fixture.path : undefined, prompt }),
    { cwd: config.workspace, env: config.env, signal: control.signal });
  const { stdout, stderr, ...metrics } = outcome;Object.assign(item, metrics);
  await writeFile(join(root, `${label}.jsonl`), stdout, { mode: 0o600 });
  await writeFile(join(root, `${label}.stderr.log`), stderr, { mode: 0o600 });
  assert.equal(outcome.timed_out, false, `${label}: CLI exceeded original ten-minute budget`);
  assert.equal(outcome.aborted, false, `${label}: interrupted`);
  assert.equal(outcome.code, 0, `${label}: CLI exit ${outcome.code}/${outcome.signal}; see ${root}`);
  const events = stdout.trim().split('\n').filter(Boolean).map(line => JSON.parse(line));
  assert.ok(events.some(e => e.type === 'turn.completed'), `${label}: real turn must complete`);
  item.answer = events.findLast(e => e.type === 'item.completed' && e.item?.type === 'agent_message')?.item.text;
  assert.equal(item.answer, fixture.expected, `${label}: expected exact uppercase fixture color`);
  item.wire = phaseEvidence(frames, label, fixture, { attachment, tool, budgets: report.describe.vision.budgets });
  item.thread = events.find(e => e.type === 'thread.started')?.thread_id;
  assert.ok(item.thread, 'CLI must report its thread identity');
  item.binding = await binding(config.env.XENOLITH_STATE_DIR, item.thread);
  item.status = 'passed';console.log(`VISION_PASS ${label}`);await flush();return item;
}
try {
  assert.equal(process.platform, 'linux', 'Real vision smoke requires Linux/WSL');
  await mkdir(join(repo, '.test-artifacts'), { recursive: true });
  root = await mkdtemp(join(repo, '.test-artifacts/vision-smoke-'));await chmod(root, 0o700);
  console.log(`VISION_ROOT ${root}`);await flush();
  const [engineBin, model, projector, codex] = await Promise.all([
    required('XENOLITH_BIN', true), required('XENOLITH_MODEL'), required('XENOLITH_MMPROJ'), required('XENOCODEX_NATIVE_CODEX_BIN', true),
  ]);enginePath = engineBin;
  const sessionControl = recoveryMode ? await required('XENOCODEX_NATIVE_SESSION_BIN', true) : undefined;
  const available = Number(/^MemAvailable:\s+(\d+)/m.exec(await readFile('/proc/meminfo', 'utf8'))?.[1]) * 1024;
  assert.ok(available >= 21 * GiB, 'Refusing model allocation: need 21 GiB MemAvailable');
  const disk = await statfs(root);assert.ok(disk.bavail * disk.bsize >= 5 * GiB, 'Require 4 GiB cache budget plus 1 GiB free');
  try { const found = await execute('pgrep', ['-x', 'xenolith']);assert.fail(`Existing Xenolith owner: ${found.stdout.trim()}`); }
  catch (error) { if (error.code !== 1) throw error; }
  for (const pid of (await readdir('/proc')).filter(p => /^\d+$/.test(p))) {
    try { assert.notEqual(await realpath(`/proc/${pid}/exe`), enginePath, `Existing engine PID ${pid}`); }
    catch (error) { if (!['ENOENT', 'ESRCH', 'EACCES', 'EPERM'].includes(error.code)) throw error; }
  }
  const globalState = process.env.XDG_STATE_HOME && isAbsolute(process.env.XDG_STATE_HOME) ? process.env.XDG_STATE_HOME : join(process.env.HOME, '.local/state');
  const lock = join(globalState, 'xenolith/engine.lock');
  await execute('python3', ['-c', 'import os,sys,fcntl\np=sys.argv[1]\nif os.path.exists(p):\n f=os.open(p,os.O_RDONLY)\n fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)\n os.close(f)', lock]);
  socketDir = await mkdtemp('/tmp/codexe-vision-');await chmod(socketDir, 0o700);
  engineSocket = join(socketDir, 'engine.sock');const clientSocket = join(socketDir, 'client.sock');
  const workspace = join(root, 'workspace');
  for (const name of ['workspace', 'home', 'tool-home', 'bindings', 'tool-bindings', 'engine-state', 'engine-cache']) await mkdir(join(root, name), { mode: 0o700 });
  const first = await imageFixture(workspace, 'fixture-a.png');
  const second = await imageFixture(workspace, 'fixture-b.png', first.expected);
  report.fixtures = { attachment: first, tool: second };
  report.config = { engine: engineBin, model, projector, codex, context: 65536, cache_budget_bytes: 4 * GiB,
    engine_socket: engineSocket, client_socket: clientSocket, shared_lock: lock, available_memory_bytes: available, disk_available_bytes: disk.bavail * disk.bsize };
  report.source = { codexe: await source(repo), engine: await source(dirname(engineBin)) };
  report.environment = { platform: process.platform, arch: process.arch, node: process.versions.node, kernel: release(), cpu: cpus()[0]?.model };
  report.sha256 = { model: await hashFile(model), cli: await hashFile(codex), engine: await hashFile(engineBin), projector: await hashFile(projector),
    runner: await hashFile(fileURLToPath(import.meta.url)), helper: await hashFile(fileURLToPath(new URL('./helpers/vision-smoke.mjs', import.meta.url))) };
  report.model = { bytes: (await stat(model)).size, name: basename(model) };
  trace = createWriteStream(join(root, 'wire.jsonl'), { mode: 0o600 });trace.on('error', error => control.abort(error));
  observer = await wireObserver(clientSocket, engineSocket, record => { frames.push(record);trace.write(JSON.stringify(record) + '\n'); });
  const env = visionEnv(process.env, { CODEX_HOME: join(root, 'home'), XENOLITH_STATE_DIR: join(root, 'bindings'),
    XENOLITH_SOCKET: clientSocket, XENOLITH_NO_SPAWN: '1', XENOLITH_MAX_OUTPUT_TOKENS: '1024' });
  const config = { engine: engineBin, model, projector, codex, workspace, env, sessionControl };
  if (sessionControl) report.sha256.session_control = await hashFile(sessionControl);
  await startEngine('initial', config);
  const original = await invoke('attachment', config, first, { attachment: true,
    prompt: 'Look at the attached image. What color is the large square inside the white border? Do not use tools. Answer with exactly one uppercase English color word and nothing else.' });
  const resumed = await invoke('resume-live', config, first, { resume: true,
    prompt: 'Recall the image in our previous turn. What color was its large square? Do not use tools. Answer with exactly one uppercase English color word and nothing else.' });
  assert.equal(resumed.binding.session, original.binding.session);
  assert.ok(resumed.binding.epoch > original.binding.epoch);
  observer.phase('snapshot-probe');
  wire = await connectWire(engineSocket, record => trace.write(JSON.stringify({ phase: 'snapshot-probe', ...record }) + '\n'), control.signal);
  const created = await wire.request({ op: 'create', system: 'Reply only OK.', tools: [] });
  await wire.request({ op: 'append', role: 'user', text: 'Say OK.' });
  report.snapshot_probe = { session: created.session, before: await wire.generate(), checkpoint: await wire.request({ op: 'checkpoint' }), verified: false };
  assert.equal(report.snapshot_probe.checkpoint.saved, true, 'Probe must have a saved snapshot');
  wire.close();wire = null;
  observer.phase('engine-restart');await stopEngine();await startEngine('restarted', config);
  const restarted = await invoke('resume-restarted', config, first, { resume: true,
    prompt: 'Using the image retained in our conversation, state its square color again. Do not use tools. Answer with exactly one uppercase English color word and nothing else.' });
  assert.equal(restarted.binding.session, original.binding.session, 'Restart must retain native conversation binding');
  assert.ok(restarted.binding.epoch > resumed.binding.epoch);
  assert.ok(restarted.wire.opened_sessions.includes(original.binding.session), 'CLI must reopen its own native session');
  wire = await connectWire(engineSocket, record => trace.write(JSON.stringify({ phase: 'snapshot-probe', ...record }) + '\n'), control.signal);
  report.snapshot_probe.open = await wire.request({ op: 'open', session: created.session });
  await wire.request({ op: 'append', role: 'user', text: 'Say OK again.' });
  report.snapshot_probe.after = await wire.generate();
  assert.equal(report.snapshot_probe.after.resume?.loaded, true, 'Independent probe must report actual snapshot loading');
  assert.ok(report.snapshot_probe.after.usage.cache_read > 0, 'Independent probe must reuse cached native tokens');
  report.snapshot_probe.verified = true;wire.close();wire = null;
  const toolConfig = { ...config, env: { ...env, CODEX_HOME: join(root, 'tool-home'), XENOLITH_STATE_DIR: join(root, 'tool-bindings') } };
  const tool = await invoke('view-image-tool', toolConfig, second, { tool: true,
    prompt: `Use view_image to open ${second.path}. Do not use shell, code, or any other tool. What color is the large square inside the white border? Answer with exactly one uppercase English color word and nothing else.` });
  assert.notEqual(tool.binding.session, original.binding.session, 'Tool case must use a fresh native conversation');
  if (recoveryMode) {
    report.sha256.recovery_helper = await hashFile(fileURLToPath(new URL('./helpers/vision-recovery.mjs', import.meta.url)));
    report.config.session_control = sessionControl;
    const { runRecovery } = await import('./helpers/vision-recovery.mjs');
    await runRecovery({ root, socketDir, engineSocket, config, report, frames, observer, invoke, original, first, second, signal: control.signal, flush });
  }
  assert.deepEqual(observer.errors, [], 'Wire observer must not hide protocol or connection errors');
  check();report.status = 'passed';console.log(`VISION_SMOKE_PASSED ${report.cases.length}/${report.cases.length}; independent native snapshot probe verified${recoveryMode ? '; recovery probes complete (see limitations)' : ''}`);
} catch (error) {
  for (const item of report.cases) if (item.status === 'running') { item.status = 'failed';item.error = error.message; }
  report.status = 'failed';report.error = error.stack;process.exitCode ||= 1;console.error(error.stack);
} finally {
  wire?.close();
  try { try { await observer?.close(); } finally { await stopEngine(); }report.cleanup = { owned_engine_stopped: true, client_groups_terminated: true }; }
  catch (error) { report.status = 'failed';report.cleanup_error = error.stack;process.exitCode ||= 1; }
  try { if (trace && !trace.destroyed) await new Promise((resolve, reject) => { trace.once('error', reject);trace.end(resolve); }); }
  catch (error) { report.status = 'failed';report.trace_error = error.stack;process.exitCode ||= 1; }
  if (control.signal.aborted) {
    report.status = 'failed';report.error ??= control.signal.reason?.stack ?? 'Vision smoke interrupted';process.exitCode ||= 1;
  }
  report.finished = new Date().toISOString();await flush();
  if (root) console.log(`Preserved vision evidence: ${root}`);
}
