// Copyright (c) 2026 Francesco Pio Crisci. SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createConnection, createServer } from 'node:net';
import { createHash, randomInt } from 'node:crypto';
import { StringDecoder } from 'node:string_decoder';
import { deflateSync } from 'node:zlib';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';

export const colors = [['RED', [230, 15, 15]], ['GREEN', [15, 190, 15]], ['BLUE', [15, 15, 230]]];
export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
export const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
export function squarePng(rgb) {
  const crc32 = bytes => {
    let crc = 0xffffffff;
    for (const byte of bytes) {
      crc ^= byte;
      for (let i = 0; i < 8; i++) crc = crc & 1 ? 0xedb88320 ^ (crc >>> 1) : crc >>> 1;
    }
    return (crc ^ 0xffffffff) >>> 0;
  };
  const chunk = (type, data) => {
    const body = Buffer.concat([Buffer.from(type), data]), size = Buffer.alloc(4), crc = Buffer.alloc(4);
    size.writeUInt32BE(data.length);crc.writeUInt32BE(crc32(body));
    return Buffer.concat([size, body, crc]);
  };
  const header = Buffer.alloc(13);header.writeUInt32BE(256);header.writeUInt32BE(256, 4);header[8] = 8;header[9] = 2;
  const pixels = Buffer.alloc(256 * 769, 255);
  for (let y = 0; y < 256; y++) {
    pixels[y * 769] = 0;
    if (y >= 32 && y < 224) for (let x = 32; x < 224; x++)
      for (let c = 0; c < 3; c++) pixels[y * 769 + 1 + x * 3 + c] = rgb[c];
  }
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', header),
    chunk('IDAT', deflateSync(pixels)), chunk('IEND', Buffer.alloc(0))]);
}
export async function imageFixture(workspace, name, exclude) {
  const choices = colors.filter(([color]) => color !== exclude), [expected, rgb] = choices[randomInt(choices.length)];
  const bytes = squarePng(rgb), path = join(workspace, name);
  await writeFile(path, bytes, { mode: 0o600 });
  return { path, expected, rgb, sha256: sha256(bytes), bytes: bytes.length };
}
export function visionEnv(base, overrides) {
  const env = { ...base, ...overrides };
  delete env.XENOLITH_REASONING_BUDGET;delete env.XENOLITH_REASONING_HISTORY;
  return env;
}
export function cliArgs({ workspace, prompt, resume = false, image }) {
  return ['exec', '--ignore-user-config', '--ignore-rules', '--skip-git-repo-check', '--sandbox', 'read-only',
    '--json', '-C', workspace, '-c', 'model_reasoning_effort="none"', ...(resume ? ['resume', '--last'] : []),
    ...(image ? ['--image', image] : []), '--', prompt];
}

// Never edit forwarded bytes. Observe complete UTF-8 frames independently.
export function frameReader(onFrame, onError, limit = 128 * 1024 * 1024) {
  const decoder = new StringDecoder('utf8');let buffer = '', failed = false;
  return bytes => {
    if (failed) return;
    try {
      buffer += decoder.write(bytes);
      for (;;) {
        const end = buffer.indexOf('\n');
        if (end < 0) { assert.ok(Buffer.byteLength(buffer) <= limit, 'Oversized observed native frame');break; }
        assert.ok(Buffer.byteLength(buffer.slice(0, end)) <= limit, 'Oversized observed native frame');
        const line = buffer.slice(0, end);buffer = buffer.slice(end + 1);
        onFrame(JSON.parse(line));
      }
    } catch (error) { failed = true;onError(error); }
  };
}
export async function wireObserver(path, upstreamPath, record) {
  const sockets = new Set(), errors = [];let sequence = 0, phase = 'startup';
  const server = createServer(client => {
    const upstream = createConnection(upstreamPath), connection = ++sequence;
    sockets.add(client);sockets.add(upstream);
    const fail = error => { errors.push({ phase, connection, error: error.message });client.destroy();upstream.destroy(); };
    for (const [socket, direction] of [[client, 'request'], [upstream, 'response']]) {
      socket.on('data', frameReader(frame => record({ at: Date.now(), phase, connection, direction, frame }), fail));
      socket.on('error', fail).on('close', () => { sockets.delete(socket);client.destroy();upstream.destroy(); });
    }
    client.pipe(upstream);upstream.pipe(client);
  });
  await new Promise((resolve, reject) => { server.once('error', reject);server.listen(path, resolve); });
  return { errors, phase(value) { phase = value; }, async close() {
    for (const socket of sockets) socket.destroy();
    await new Promise(resolve => server.close(resolve));
  } };
}
export function phaseEvidence(records, phase, fixture, { tool = false, attachment = false, budgets = [] } = {}) {
  const frames = records.filter(r => r.phase === phase), images = [], sessionAt = new Map(), sessions = new Map();
  for (const record of records) {
    if (record.direction === 'request' && record.frame.op === 'open') sessions.set(record.connection, record.frame.session);
    if (record.direction === 'response' && typeof record.frame.session === 'string') sessions.set(record.connection, record.frame.session);
    sessionAt.set(record, sessions.get(record.connection));
  }
  for (const record of frames.filter(r => r.direction === 'request')) {
    const request = record.frame;
    for (const message of request.op === 'rebuild' ? request.messages : request.op === 'append' ? [request] : []) {
      if (message.image?.data) images.push({ connection: record.connection, op: request.op, role: message.role,
        call_id: message.call_id, session: sessionAt.get(record), budget: message.image.budget, sha256: sha256(Buffer.from(message.image.data, 'base64')) });
    }
  }
  const calls = frames.filter(r => r.direction === 'response' && r.frame.event === 'toolcall_end');
  if (attachment || tool) {
    assert.ok(images.some(i => i.sha256 === fixture.sha256 && budgets.includes(i.budget)), `${phase}: actual fixture bytes with advertised image budget must reach the native engine`);
    assert.ok(images.every(i => i.sha256 === fixture.sha256), `${phase}: fresh conversation must not contain another fixture`);
  }
  if (tool) {
    assert.equal(calls.length, 1, `${phase}: exactly one actual tool call required`);
    const call = calls[0];
    assert.equal(call.frame.name, 'view_image');
    assert.equal(call.frame.arguments.path, fixture.path);
    assert.ok(images.some(i => i.role === 'tool' && i.call_id === call.frame.id && i.session && i.session === sessionAt.get(call)
      && i.sha256 === fixture.sha256), `${phase}: executed tool result must return the fixture image bytes for the native call ID`);
    const firstTool = frames.indexOf(call);
    assert.ok(!frames.slice(0, firstTool).some(r => r.direction === 'request' &&
      (r.frame.image || r.frame.messages?.some(m => m.image))), `${phase}: image must come from view_image, not a seeded attachment`);
  } else assert.equal(calls.length, 0, `${phase}: no tool execution expected`);
  const terminals = frames.filter(r => r.direction === 'response' && r.frame.event === 'done').map(r => r.frame);
  assert.ok(terminals.some(t => t.stop === 'stop'), `${phase}: actual native generation must complete`);
  return { images, tool_calls: calls.map(r => r.frame), terminals, opened_sessions: frames
    .filter(r => r.direction === 'request' && r.frame.op === 'open').map(r => r.frame.session) };
}

// Each child owns its Unix process group; stdin is closed even for --image.
export async function runClient(binary, args, { cwd, env, signal, timeoutMs = 600000 } = {}) {
  signal?.throwIfAborted();
  const child = spawn(binary, args, { cwd, env, detached: process.platform !== 'win32', stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '', stderr = '', timed_out = false, killTimer;
  const started = performance.now();
  child.stdout.on('data', b => { stdout += b; });child.stderr.on('data', b => { stderr += b; });
  const kill = sig => {
    try { if (process.platform !== 'win32' && child.pid) process.kill(-child.pid, sig);else child.kill(sig); }
    catch (error) { if (error.code !== 'ESRCH') throw error; }
  };
  const terminate = () => { kill('SIGTERM');killTimer ??= setTimeout(() => kill('SIGKILL'), 1000); };
  const aborted = () => terminate();signal?.addEventListener('abort', aborted, { once: true });
  const timer = setTimeout(() => { timed_out = true;terminate(); }, timeoutMs);
  try {
    const result = await new Promise((resolve, reject) => {
      child.once('error', reject);child.once('close', (code, signal) => resolve({ code, signal }));
    });
    return { ...result, pid: child.pid, timed_out, aborted: signal?.aborted ?? false,
      elapsed_ms: performance.now() - started, stdout, stderr };
  } finally {
    clearTimeout(timer);clearTimeout(killTimer);signal?.removeEventListener('abort', aborted);
    if (timed_out || signal?.aborted) kill('SIGKILL');
  }
}

// Direct probe connection; it never operates on the CLI's bound sessions.
export async function connectWire(path, record, signal) {
  signal?.throwIfAborted();
  const socket = createConnection(path), queue = [], pending = [];
  let failure;
  const fail = error => { failure ??= error;for (const waiter of pending.splice(0)) waiter.reject(failure); };
  const abort = () => socket.destroy(new Error('Native probe interrupted'));
  signal?.addEventListener('abort', abort, { once: true });
  socket.on('data', frameReader(frame => {
    record({ direction: 'response', frame });
    if (pending.length) pending.shift().resolve(frame);else queue.push(frame);
  }, error => socket.destroy(error)));
  socket.on('error', fail).on('close', () => { signal?.removeEventListener('abort', abort);fail(new Error('Native probe closed')); });
  await new Promise((resolve, reject) => { socket.once('connect', resolve);socket.once('error', reject); });
  async function receive(timeoutMs = 300000) {
    if (failure) throw failure;
    let timer;
    try {
      return await Promise.race([queue.length ? Promise.resolve(queue.shift()) : new Promise((resolve, reject) => pending.push({ resolve, reject })),
        new Promise((_, reject) => { timer = setTimeout(() => { socket.destroy();reject(new Error('Native probe timeout')); }, timeoutMs); })]);
    } finally { clearTimeout(timer); }
  }
  async function send(frame) { signal?.throwIfAborted();record({ direction: 'request', frame });socket.write(JSON.stringify(frame) + '\n'); }
  async function checked() {
    const frame = await receive();assert.ok(frame.ok !== false && frame.event !== 'error', JSON.stringify(frame));return frame;
  }
  return { async request(frame, timeoutMs) {
    await send(frame);const response = timeoutMs ? await receive(timeoutMs) : await checked();assert.equal(response.ok, true, JSON.stringify(response));return response;
  }, async generate() {
    await send({ op: 'generate', reasoning: false, max_tokens: 8 });
    for (;;) { const frame = await checked();if (frame.event === 'done') return frame; }
  }, close() { socket.destroy(); } };
}
