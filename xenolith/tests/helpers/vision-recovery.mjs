// Copyright (c) 2026 Francesco Pio Crisci. SPDX-License-Identifier: Apache-2.0
// Opt-in real recovery probes. Fault injection is explicitly distinct from observation.
import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { randomBytes } from 'node:crypto';
import { createConnection, createServer } from 'node:net';
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { cliArgs, frameReader, imageFixture, runClient, sha256, sleep } from './vision-smoke.mjs';

const execute = promisify(execFile);
const recall = 'Recall the first image in this conversation. What color is the large square inside its white border? Do not use tools. Answer with exactly one uppercase English color word and nothing else.';
export function branchAnswer(items) {
  assert.ok(items.every(i => ['userMessage', 'agentMessage', 'reasoning'].includes(i.type)), 'No tool effects expected in branch fixtures');
  return items.findLast(i => i.type === 'agentMessage')?.text;
}

const inspect = 'What color is the large square inside the white border in this attached image? Do not use tools. Answer with exactly one uppercase English color word and nothing else.';

// Forward frames semantically unchanged, except the explicitly selected lost ACK.
// This proxy is used only for fault injection, never as a transparent observer.
export async function faultProxy(path, upstreamPath, { mode, record, interrupt }) {
  const sockets = new Set(), errors = [], faults = [];let sequence = 0, armed = true, closing;
  const server = createServer(client => {
    const upstream = createConnection(upstreamPath), connection = `fault-${++sequence}`;
    let waitingImage = false, faultTimer;
    const fail = error => { errors.push(error.message);client.destroy();upstream.destroy(); };
    for (const socket of [client, upstream]) {
      sockets.add(socket);
      socket.on('error', fail).on('close', () => {
        clearTimeout(faultTimer);sockets.delete(socket);client.destroy();upstream.destroy();
      });
    }
    client.on('data', frameReader(frame => {
      record({ at: Date.now(), connection, direction: 'request', frame });
      const images = frame.op === 'rebuild' ? frame.messages?.filter(m => m.image) : frame.image ? [frame] : [];
      if (armed && images?.length && (mode === 'before-image-ack' || frame.role === 'tool')) {
        waitingImage = true;
        if (mode === 'before-image-ack') faultTimer = setTimeout(() => {
          if (!armed) return;
          armed = false;faults.push({ kind: mode, connection, request: frame, at: Date.now() });interrupt();
        }, 20);
      }
      upstream.write(JSON.stringify(frame) + '\n');
    }, fail));
    upstream.on('data', frameReader(frame => {
      record({ at: Date.now(), connection, direction: 'response', frame });
      if (waitingImage && typeof frame.ok === 'boolean') {
        waitingImage = false;
        if (armed && mode === 'lost-tool-image-ack' && frame.ok) {
          armed = false;faults.push({ kind: mode, connection, ack: frame, at: Date.now() });interrupt();return;
        }
        if (armed && mode === 'before-image-ack') {
          clearTimeout(faultTimer);armed = false;
          errors.push('Image reply arrived before the intended interruption');
        }
      }
      client.write(JSON.stringify(frame) + '\n');
    }, fail));
  });
  await new Promise((resolve, reject) => { server.once('error', reject);server.listen(path, resolve); });
  return { faults, errors, async close() {
    closing ??= (async () => {
      armed = false;
      for (const socket of sockets) socket.destroy();
      await new Promise(resolve => server.close(resolve));
    })();
    await closing;
  } };
}

async function storedBinding(directory, thread) {
  return JSON.parse(await readFile(join(directory, `${thread}.json`), 'utf8'));
}
async function privateConfig(root, config, name) {
  const home = join(root, `${name}-home`), bindings = join(root, `${name}-bindings`);
  await mkdir(home, { mode: 0o700 });await mkdir(bindings, { mode: 0o700 });
  return { ...config, env: { ...config.env, CODEX_HOME: home, XENOLITH_STATE_DIR: bindings } };
}
function imageMessages(records) {
  return records.filter(r => r.direction === 'request').flatMap(r =>
    r.frame.op === 'rebuild' ? r.frame.messages : r.frame.op === 'append' ? [r.frame] : []).filter(m => m.image);
}

async function appServer(config, root, signal) {
  const child = spawn(config.codex, ['--no-daemon', '-c', 'model_reasoning_effort="none"', 'app-server', '--stdio'],
    { cwd: config.workspace, env: config.env, stdio: ['pipe', 'pipe', 'pipe'] });
  const messages = [], pending = new Map();let nextId = 0, stderr = '', failure;
  const fail = error => { failure ??= error;for (const waiter of pending.values()) waiter.reject(error); };
  child.stdout.on('data', frameReader(message => {
    messages.push(message);
    if (message.id !== undefined && message.method) {
      // Fixtures use no tools or approvals. Unexpected server requests must fail.
      fail(new Error(`Unexpected app-server request ${message.method}`));return;
    }
    pending.get(message.id)?.resolve(message);
  }, fail));
  child.stderr.on('data', b => { stderr += b; });child.on('error', fail);
  const exited = new Promise(resolve => child.once('close', (code, signal) => {
    fail(new Error(`App-server exited ${code}/${signal}`));resolve({ code, signal });
  }));
  const abort = () => child.kill('SIGTERM');signal.addEventListener('abort', abort, { once: true });
  return { messages, notify(method) { child.stdin.write(JSON.stringify({ method }) + '\n'); }, async request(method, params) {
    if (failure) throw failure;signal.throwIfAborted();const id = ++nextId;let timer;
    try {
      const message = await new Promise((resolve, reject) => {
        timer = setTimeout(() => reject(new Error(`App-server timeout: ${method}`)), 600000);
        pending.set(id, { resolve, reject });child.stdin.write(JSON.stringify({ id, method, params }) + '\n');
      });
      assert.ok(!message.error, JSON.stringify(message));return message.result;
    } finally { clearTimeout(timer);pending.delete(id); }
  }, async turn(thread, input) {
    const offset = messages.length;
    const result = await this.request('turn/start', { threadId: thread, input, effort: 'none' });
    const deadline = Date.now() + 600000;
    for (;;) {
      signal.throwIfAborted();if (failure) throw failure;
      const done = messages.slice(offset).find(m => m.method === 'turn/completed' && m.params.threadId === thread && m.params.turn.id === result.turn.id);
      if (done) {
        assert.equal(done.params.turn.status, 'completed', JSON.stringify(done));
        const items = messages.slice(offset).filter(m => m.method === 'item/completed' && m.params.threadId === thread).map(m => m.params.item);
        return { id: result.turn.id, answer: branchAnswer(items) };
      }
      assert.ok(Date.now() < deadline, 'App-server turn timeout');await sleep(50);
    }
  }, async close() {
    signal.removeEventListener('abort', abort);child.stdin.end();child.kill('SIGTERM');
    const timer = setTimeout(() => child.kill('SIGKILL'), 5000);
    try { await exited; } finally {
      clearTimeout(timer);
      await writeFile(join(root, 'fork-app-server.json'), JSON.stringify(messages, null, 2), { mode: 0o600 });
      await writeFile(join(root, 'fork-app-server.stderr.log'), stderr, { mode: 0o600 });
    }
  } };
}

export async function runRecovery(context) {
  const { root, socketDir, engineSocket, config, report, frames, observer, invoke, original, first, second, signal, flush } = context;
  report.recovery = { status: 'running', limitations: [
    'Engine v2 image preparation is synchronous. Client interruption before its ACK is not proof of encoder cancellation; cooperative encoding cancellation needs engine support.',
    'Conversation branch/rewind recovery is distinct from actual KV loading, verified only by the separate snapshot probe.',
  ], probes: [] };
  const probe = async (label, action) => {
    signal.throwIfAborted();const item = { label, status: 'running' };report.recovery.probes.push(item);await flush();
    try { Object.assign(item, await action());item.status = 'passed';console.log(`RECOVERY_PASS ${label}`); }
    catch (error) { item.status = 'failed';report.recovery.status = 'failed';item.error = error.stack;throw error; }
    finally { await flush(); }
  };

  await probe('rewind-retained-image', async () => {
    const files = await readdir(join(config.env.CODEX_HOME, 'sessions'), { recursive: true });
    const rollout = files.find(path => path.endsWith('.jsonl') && path.includes(original.thread));assert.ok(rollout);
    observer.phase('offline-rewind');
    const args = [engineSocket, config.env.XENOLITH_STATE_DIR, original.thread, String(original.binding.marker), join(config.env.CODEX_HOME, 'sessions', rollout)];
    const applied = JSON.parse((await execute(config.sessionControl, args, { signal, timeout: 600000 })).stdout);
    assert.equal(applied.rewound, true);assert.ok(applied.removed_user_turns >= 2);
    const resumed = await invoke('rewind-resume', config, first, { resume: original.thread, prompt: recall });
    assert.equal(resumed.binding.session, original.binding.session);
    assert.ok(!JSON.stringify(resumed.binding.history).includes('Using the image retained in our conversation'), 'Discarded turn must stay removed');
    return { applied, session: resumed.binding.session };
  });

  await probe('image-budget-rebuild', async () => {
    const budgets = report.describe.vision.budgets.filter(b => b !== original.binding.image_budget);
    assert.ok(budgets.length);const budget = budgets[0];
    const changed = { ...config, env: { ...config.env, XENOLITH_IMAGE_BUDGET: String(budget) } };
    const result = await invoke('budget-changed', changed, first, { resume: original.thread, prompt: recall });
    assert.equal(result.binding.session, original.binding.session);assert.equal(result.binding.image_budget, budget);
    const messages = imageMessages(frames.filter(r => r.phase === 'budget-changed'));
    assert.ok(messages.length);assert.ok(messages.every(m => m.image.budget === budget && sha256(Buffer.from(m.image.data, 'base64')) === first.sha256));
    assert.ok(frames.some(r => r.phase === 'budget-changed' && r.direction === 'request' && r.frame.op === 'rebuild'));
    const restored = await invoke('budget-restored', config, first, { resume: original.thread, prompt: recall });
    assert.equal(restored.binding.image_budget, original.binding.image_budget);
    return { changed_budget: budget, restored_budget: restored.binding.image_budget, session: restored.binding.session };
  });

  for (const mode of ['lost-tool-image-ack', 'before-image-ack']) await probe(mode, async () => {
    const privateCase = await privateConfig(root, config, mode), records = [];let child;
    const fixture = mode === 'before-image-ack'
      ? await imageFixture(config.workspace, 'interrupted-image.png', undefined, randomBytes(16).toString('hex')) : second;
    if (mode === 'before-image-ack') report.recovery.interrupted_fixture = fixture;
    const fault = await faultProxy(join(socketDir, `${mode}.sock`), engineSocket, { mode,
      record: r => records.push(r), interrupt() { assert.ok(child && child.exitCode === null);child.kill('SIGKILL'); } });
    try {
      const prompt = mode === 'lost-tool-image-ack' ? `Use view_image to open ${fixture.path}. Do not use shell, code, or any other tool. What color is the large square inside the white border? Answer with exactly one uppercase English color word and nothing else.` : inspect;
      const env = { ...privateCase.env, XENOLITH_SOCKET: join(socketDir, `${mode}.sock`),
        XENOLITH_IMAGE_BUDGET: String(Math.min(...report.describe.vision.budgets)) };
      const outcome = await runClient(config.codex, ['--no-daemon', ...cliArgs({ workspace: config.workspace, prompt, image: mode === 'before-image-ack' ? fixture.path : undefined })],
        { cwd: config.workspace, env, signal, onSpawn(value) { child = value; } });
      await writeFile(join(root, `${mode}.jsonl`), outcome.stdout, { mode: 0o600 });
      await writeFile(join(root, `${mode}.stderr.log`), outcome.stderr, { mode: 0o600 });
      assert.equal(outcome.timed_out, false);assert.equal(outcome.aborted, false);assert.equal(outcome.signal, 'SIGKILL');
      assert.equal(fault.faults.length, 1);assert.deepEqual(fault.errors, []);
      const thread = outcome.stdout.split('\n').filter(Boolean).map(JSON.parse).find(e => e.type === 'thread.started')?.thread_id;assert.ok(thread);
      const before = await storedBinding(env.XENOLITH_STATE_DIR, thread);assert.equal(before.in_flight, true);
      if (mode === 'lost-tool-image-ack') {
        assert.equal(before.pending.length, 1);
        const call = records.find(r => r.direction === 'response' && r.frame.event === 'toolcall_end');
        assert.equal(call?.frame.name, 'view_image');assert.equal(call.frame.arguments.path, fixture.path);
        const images = imageMessages(records);assert.equal(images.length, 1);assert.equal(images[0].call_id, call.frame.id);
        assert.equal(sha256(Buffer.from(images[0].image.data, 'base64')), fixture.sha256);
      }
      await fault.close();
      // Reconcile through the actual retained Codex rollout, never by editing bindings.
      const resumed = await invoke(`${mode}-resume`, privateCase, fixture, { resume: thread,
        prompt: 'Recall the image already retained in our conversation. Do not call any tools. State the square color as exactly one uppercase English word.' });
      assert.equal(resumed.binding.session, before.session);assert.deepEqual(resumed.binding.pending, []);
      if (mode === 'lost-tool-image-ack') {
        assert.equal(records.filter(r => r.direction === 'response' && r.frame.event === 'toolcall_end').length, 1);
        assert.equal(resumed.binding.history.filter(item => item.type === 'function_call_output').length, 1, 'One retained tool result; no duplicate image-tool outcome');
      }
      return { thread, session: before.session, crashed_client: { pid: outcome.pid, code: outcome.code, signal: outcome.signal }, fault: fault.faults[0], in_flight_before: before.in_flight,
        pending_before: before.pending.length, recovery_epoch: resumed.binding.epoch, evidence_scope: mode === 'before-image-ack' ? 'Client interruption before image reply; encoder cancellation unverified' : 'Image tool append accepted by engine; ACK lost; no repeated tool call on recovery' };
    } finally {
      await writeFile(join(root, `${mode}-fault-wire.json`), JSON.stringify(records, null, 2), { mode: 0o600 });
      await fault.close();
    }
  });

  await probe('fork-isolates-retained-images', async () => {
    const rpc = await appServer(config, root, signal);
    let fork, turn, after;
    try {
      await rpc.request('initialize', { clientInfo: { name: 'codexe-real-vision-recovery', version: '1' }, capabilities: { experimentalApi: true } });
      rpc.notify('initialized');
      fork = await rpc.request('thread/fork', { threadId: original.thread, cwd: config.workspace, approvalPolicy: 'never', sandbox: 'read-only' });
      assert.notEqual(fork.thread.id, original.thread);observer.phase('fork-recall');
      turn = await rpc.turn(fork.thread.id, [{ type: 'text', text: recall }]);assert.equal(turn.answer, first.expected);
      assert.ok(imageMessages(frames.filter(r => r.phase === 'fork-recall')).some(m => sha256(Buffer.from(m.image.data, 'base64')) === first.sha256), 'Fork must actually replay retained image bytes');
      const forkBefore = await storedBinding(config.env.XENOLITH_STATE_DIR, fork.thread.id);
      assert.notEqual(forkBefore.session, original.binding.session);
      observer.phase('fork-new-image');
      after = await rpc.turn(fork.thread.id, [{ type: 'localImage', path: second.path }, { type: 'text', text: inspect }]);
      assert.equal(after.answer, second.expected);
      const requests = imageMessages(frames.filter(r => r.phase === 'fork-new-image'));
      assert.ok(requests.some(m => sha256(Buffer.from(m.image.data, 'base64')) === second.sha256));
    } finally { await rpc.close(); }
    const parent = await invoke('parent-after-fork', config, first, { resume: original.thread, prompt: recall });
    assert.equal(parent.binding.session, original.binding.session);
    assert.ok(!JSON.stringify(parent.binding.history).includes((await readFile(second.path)).toString('base64')), 'Parent must not inherit fork image bytes');
    const forkBinding = await storedBinding(config.env.XENOLITH_STATE_DIR, fork.thread.id);
    assert.notEqual(forkBinding.session, parent.binding.session);assert.equal(forkBinding.in_flight, false);
    assert.deepEqual(forkBinding.pending, []);
    return { parent_thread: original.thread, fork_thread: fork.thread.id, parent_session: parent.binding.session,
      fork_session: forkBinding.session, fork_initial_answer: turn.answer, fork_changed_answer: after.answer, parent_answer: parent.answer };
  });
  report.recovery.status = 'passed_with_engine_limitations';await flush();
}
